//! Synchronous officer coordination with live actor queries.

use super::*;
use crate::ai::{
    AiEntityHandle, AiState, EmoticonType, Position, Stimulus, StimulusInfo, Substate,
};
use crate::ai_enemy::{EnemyAi, task_priority};
use crate::element::{Element as _, Human as _};
use crate::profiles::ProfileRank;
use crate::sim_rng::SimulationContext;

fn officer_report_in_progress(substate: Substate) -> bool {
    matches!(
        substate,
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
    )
}

struct AlertExecution<'a> {
    engine: &'a mut EngineInner,
    sim: &'a SimulationContext,
    assets: &'a LevelAssets,
    owner: EntityId,
}

impl EngineInner {
    pub(in crate::engine) fn execute_ai_officer_instruct_group(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        use crate::ai::{Hint, ReportType};
        use crate::ai_enemy::SeekFlags;
        let enemy = self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("officer group instruction"));
        let mut instruction = Hint {
            seek_point: enemy.base.seek_position,
            who_tells_me: AiEntityHandle::new(owner.index()),
            seek_flags: SeekFlags::REPORT_OFFICER_AFTER.bits(),
        };
        let position = self.live_ai_position(owner);
        if (instruction.seek_point.x - position.x)
            .abs()
            .max((instruction.seek_point.y - position.y).abs())
            > 100.0
        {
            instruction.seek_flags |= SeekFlags::LOCATION_FIRST.bits();
        }
        let mut count = enemy.alerted_us.len();
        let path_owner = if enemy.base.my_reconnaissance_report.report_type
            == ReportType::MissedCharly
        {
            instruction.seek_flags |= SeekFlags::CHARLY_SEEK.bits();
            let charly = enemy
                .base
                .my_reconnaissance_report
                .charly
                .expect("group checkpoint report requires a checkpoint");
            let charly = self.expect_human_id_for_ai_handle(charly.get(), "group checkpoint path");
            let ai = self
                .world
                .entities
                .expect_ai_controller(charly, format_args!("group checkpoint path"));
            if ai.has_patrol_path && count > 0 {
                instruction.seek_flags |= SeekFlags::LOCATION_FIRST.bits();
                Some(charly)
            } else {
                None
            }
        } else {
            None
        };
        let path_index = |engine: &EngineInner, charly| {
            let ai = engine
                .world
                .entities
                .expect_ai_controller(charly, format_args!("group checkpoint path"));
            ai.patrol_path
                .as_ref()
                .map(|path| path.hiking_path_index)
                .or(ai.detached_patrol_path_status.hiking_path_index)
                .expect("checkpoint with a path requires its authored path")
                .get() as usize
        };
        let path_size = path_owner.map_or(0, |charly| {
            assets.navigation.hiking_paths[path_index(self, charly)]
                .waypoints
                .len()
        });
        assert!(
            path_owner.is_none() || path_size > 0,
            "group checkpoint path is empty"
        );
        let waypoint_step = if path_owner.is_some() && count > 1 {
            (path_size - 1) / (count - 1)
        } else {
            0
        };
        let mut waypoint_index = 0;
        let mut index = 0;
        while index < count {
            if let Some(charly) = path_owner {
                // Assignment replaces the checkpoint's path contents during
                // recipient callbacks; the cursor and stride remain local.
                let path = path_index(self, charly);
                let waypoint = &assets.navigation.hiking_paths[path].waypoints[waypoint_index];
                instruction.seek_point = Position {
                    x: waypoint.x as f32,
                    y: waypoint.y as f32,
                    sector: assets.navigation.hiking_waypoint_sector(
                        path,
                        waypoint_index,
                        waypoint.sector,
                    ),
                    level: waypoint.level,
                };
                waypoint_index = (waypoint_index + waypoint_step) % path_size;
            } else if index > 0 {
                instruction.seek_flags &= !SeekFlags::LOCATION_FIRST.bits();
            }
            let target = self
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("group instruction recipient"))
                .alerted_us[index];
            let target = self.expect_human_id_for_ai_handle(target, "group instruction recipient");
            let mut stimulus = Stimulus::new(StimulusType::CallInstruction);
            stimulus.info = StimulusInfo::Hint(instruction);
            if self.execute_ai_callback(sim, assets, target, &stimulus) {
                index += 1;
            } else {
                self.world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("refused group instruction"))
                    .alerted_us
                    .remove(index);
                count -= 1;
            }
        }
        if count > 0 {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Seeking,
                Substate::SeekingOfficerWaitForInstructedGroup,
            );
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("group instruction timer"))
                .launch_timer(30, self.control.frame_counter);
        } else {
            self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::empty());
        }
    }

    pub(in crate::engine) fn execute_ai_run_and_alert_soldiers(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
    ) -> bool {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .run_and_alert_soldiers(center)
    }
    pub(in crate::engine) fn execute_ai_officer_look_for_soldier(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        reason: crate::ai::ReportType,
    ) {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .officer_look_for_soldier(reason);
    }
    pub(in crate::engine) fn execute_ai_tower_guard_alert(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
    ) {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .tower_guard_alert(center);
        self.execute_battle_decisions(sim, assets, owner);
    }
    pub(in crate::engine) fn execute_ai_alert_officer_for_caller(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        caller: crate::ai::OfficerAlertCaller,
    ) {
        use crate::ai::OfficerAlertCaller;
        let accepted = self.execute_ai_alert_officer(sim, assets, owner);
        match caller {
            OfficerAlertCaller::Ignore => return,
            OfficerAlertCaller::TowerGuardCalled => {
                self.world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("tower guard alert priority"))
                    .current_task_priority = task_priority::ALERT_IGNORE_ENEMY;
                return;
            }
            OfficerAlertCaller::ReturnToDuty => {
                if !accepted {
                    self.execute_ai_return_to_duty(
                        sim,
                        assets,
                        owner,
                        crate::ai::DutyFlags::empty(),
                    );
                }
                return;
            }
            _ if accepted => return,
            _ => {}
        }
        match caller {
            OfficerAlertCaller::SeekBody { center, radius } => self.execute_failed_ai_alert(
                sim,
                assets,
                owner,
                crate::ai::AlertSoldiersFailureContinuation::SeekBody { center, radius },
            ),
            OfficerAlertCaller::SeekMissedCharly { .. } => self.execute_failed_ai_alert(
                sim,
                assets,
                owner,
                crate::ai::AlertSoldiersFailureContinuation::SeekMissedCharly {
                    center: self.live_ai_position(owner),
                },
            ),
            OfficerAlertCaller::SeekHint { center } => self.execute_ai_seek_area(
                sim,
                assets,
                owner,
                center,
                crate::parameters_ai::AI_HINT_SEEK_RADIUS as u16,
                crate::ai_enemy::SeekFlags::LOCATION_FIRST,
                crate::ai_enemy::UNDEFINED_DIRECTION,
            ),
            _ => unreachable!(),
        }
    }
    pub(in crate::engine) fn execute_ai_alert_officer(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) -> bool {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .alert_officer()
    }
    pub(in crate::engine) fn execute_ai_alert_soldiers_with_failure(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
        flags: u16,
        failure: crate::ai::AlertSoldiersFailureContinuation,
    ) {
        if self.execute_ai_alert_soldiers(sim, assets, owner, center, flags) {
            return;
        }
        self.execute_failed_ai_alert(sim, assets, owner, failure);
    }

    fn execute_failed_ai_alert(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        failure: crate::ai::AlertSoldiersFailureContinuation,
    ) {
        use crate::ai::AlertSoldiersFailureContinuation as Failure;
        use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
        let (center, radius, flags) = match failure {
            Failure::None => return,
            Failure::ReturnToDuty => {
                self.execute_ai_return_to_duty(sim, assets, owner, crate::ai::DutyFlags::empty());
                return;
            }
            Failure::SeekBody { center, radius } => (
                center,
                radius,
                SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
            ),
            Failure::SeekMissedCharly { .. } => {
                let checkpoint = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("missing checkpoint seek"))
                    .checkpoint_charly
                    .expect("missing checkpoint seek requires a checkpoint");
                let checkpoint =
                    self.expect_human_id_for_ai_handle(checkpoint.get(), "missing checkpoint seek");
                let has_path = self
                    .world
                    .entities
                    .expect_ai_controller(checkpoint, format_args!("missing checkpoint path"))
                    .has_patrol_path;
                (
                    self.live_ai_position(owner),
                    if has_path {
                        crate::parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
                    } else {
                        crate::parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
                    },
                    SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
                )
            }
            Failure::FleeingRunToDoor => {
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Fleeing,
                    Substate::FleeingRunToDoor,
                );
                self.execute_ai_callback(
                    sim,
                    assets,
                    owner,
                    &Stimulus::new(StimulusType::EventReachPoint),
                );
                return;
            }
        };
        self.execute_ai_seek_area(
            sim,
            assets,
            owner,
            center,
            radius,
            flags,
            UNDEFINED_DIRECTION,
        );
    }

    pub(in crate::engine) fn execute_ai_alert_soldiers(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
        flags: u16,
    ) -> bool {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .alert_soldiers(center, flags)
    }
    pub(in crate::engine) fn execute_maybe_officer_sees_me_fighting(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        if self
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("brawl owner"))
            .soldier_profile_rank
            != ProfileRank::Soldier
        {
            return;
        }
        let camp = self.expect_entity(owner, "brawl camp").camp();
        let count = self.ai.global.all_soldier_handles.len();
        for index in 0..count {
            let handle = self.ai.global.all_soldier_handles[index];
            let candidate = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            let Some(Entity::Soldier(soldier)) = self.world.entities.get(candidate) else {
                continue;
            };
            if soldier.soldier.cached_camp != camp {
                continue;
            }
            let enemy = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("officer candidate has no brain");
            if enemy.soldier_profile_rank != ProfileRank::Officer
                || !(enemy.base.current_state == AiState::Default
                    || enemy.base.current_substate == Substate::WonderingMoneyReactiontime)
            {
                continue;
            }
            let observer = soldier.element.position();
            let actor = self
                .expect_entity(owner, "brawl actor")
                .element_data()
                .position();
            let dx = observer.x - actor.x;
            let dy = (observer.y - actor.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = observer.z - actor.z;
            let distance = dx * dx + dy * dy + dz * dz;
            let reacts = if distance < 200.0 * 200.0 {
                true
            } else if distance < 350.0 * 350.0 {
                self.live_ai_detects_180(assets, candidate, owner)
            } else {
                self.npc_is_detecting_human(assets, candidate, owner, self.control.frame_counter)
            };
            if reacts {
                let mut stimulus = Stimulus::new(StimulusType::EventSeesBrawl);
                stimulus.info = StimulusInfo::Human(AiEntityHandle::new(owner.index()));
                self.execute_ai_callback(sim, assets, candidate, &stimulus);
                return;
            }
        }
    }

    pub(in crate::engine) fn execute_ai_command_soldiers_to_attack(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
    ) -> bool {
        AlertExecution {
            engine: self,
            sim,
            assets,
            owner,
        }
        .command_soldiers_to_attack(center)
    }

    pub(in crate::engine) fn execute_ai_combat_alert_decision(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        center: Position,
    ) {
        let accepted = self.execute_ai_command_soldiers_to_attack(sim, assets, owner, center);
        if accepted {
            self.owner_work_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::OfficerGivesAttackOrder,
                    flags: 0,
                },
            );
        } else {
            self.execute_ai_battle_reserve(sim, assets, owner);
        }
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("combat alert decision log"))
            .register_log_line(
                crate::ai::LogLineType::BattleDecision,
                if accepted {
                    crate::ai::Decision::AlertSoldiers
                } else {
                    crate::ai::Decision::Reserve
                } as u16,
            );
    }
}

#[cfg(test)]
mod tests;

impl AlertExecution<'_> {
    fn run_and_alert_soldiers(&mut self, center: Position) -> bool {
        self.enemy_mut().base.outbox.actor.set_unfocus();
        self.settle();
        self.enemy_mut().base.seek_position = center;
        let owner = self
            .engine
            .expect_entity(self.owner, "reservist search owner");
        let camp = owner.camp();
        let auth = owner.actor_auth_info();
        let position = owner.element_data().position_map();
        let layer = owner.element_data().layer();
        let mut best = None;
        let mut minimum = u32::MAX as f32;
        for (index, door) in self
            .engine
            .script_domains
            .interactables
            .doors
            .iter()
            .enumerate()
        {
            if door.door_type != crate::gate::DoorType::Building {
                continue;
            }
            let house = self
                .engine
                .ai
                .global
                .houses
                .iter()
                .find(|house| house.sector_index == u32::from(u16::from(door.sector_in)))
                .expect("building door has no house");
            let building = house
                .building_index
                .expect("building door has no building identity");
            let occupants = self
                .engine
                .script_domains
                .buildings
                .occupants
                .get(usize::from(building))
                .expect("building occupants are missing");
            let mut reservists = 0u16;
            for &handle in occupants {
                let id = self
                    .engine
                    .entity_id_for_actor_handle(handle)
                    .expect("building occupant is missing");
                let Entity::Soldier(soldier) = self.engine.expect_entity(id, "reservist occupant")
                else {
                    continue;
                };
                if soldier.soldier.cached_camp == camp
                    && soldier
                        .npc
                        .ai_brain
                        .enemy()
                        .expect("reservist brain")
                        .soldier_profile_rank
                        == ProfileRank::Soldier
                    && soldier.is_able_to_help()
                {
                    reservists += 1;
                }
            }
            if reservists < 3
                || !door.is_actor_authorized(
                    true,
                    &auth,
                    self.engine.building_sector_is_authorized(door.sector_in),
                    false,
                )
            {
                continue;
            }
            let mut distance = (door.point_out.x - position.x)
                .abs()
                .max((door.point_out.y - position.y).abs());
            if door.layer_out != layer {
                distance += 1000.0;
            }
            distance /= f32::from(reservists);
            if distance < minimum {
                minimum = distance;
                best = Some(index);
            }
        }
        let Some(index) = best else {
            return false;
        };
        let door = &self.engine.script_domains.interactables.doors[index];
        let position = Position {
            x: door.point_in.x,
            y: door.point_in.y,
            level: door.layer_in,
            sector: crate::position_interface::SectorHandle::new(u16::from(door.sector_in)).map(
                |handle| {
                    handle.with_arena_index(door.sector_in_index.expect("reservist door sector"))
                },
            ),
        };
        self.enemy_mut().base.my_door_index =
            Some(crate::gate::DoorIndex::new(index as u32).expect("door index"));
        self.state(AiState::Fleeing, Substate::FleeingRunToAlertSoldiers);
        self.engine.duty_go_to(
            self.sim,
            self.assets,
            self.owner,
            position,
            crate::ai::GotoFlags::RUN,
        );
        true
    }
    fn officer_look_for_soldier(&mut self, reason: crate::ai::ReportType) {
        assert_eq!(self.enemy().soldier_profile_rank, ProfileRank::Officer);
        let head = self.enemy().base.patrol.first().copied().filter(|id| {
            matches!(self.engine.world.entities.get(*id), Some(Entity::Soldier(soldier))
                if soldier.npc.ai_brain.enemy().expect("patrol soldier brain").soldier_profile_rank == ProfileRank::Soldier)
        });
        let camp = self.engine.expect_entity(self.owner, "officer camp").camp();
        let registry: Vec<_> = self
            .engine
            .ai
            .global
            .all_soldier_handles
            .iter()
            .copied()
            .map(|handle| EntityId::Soldier(crate::entity_id::SoldierId(handle)))
            .filter(|id| {
                self.engine
                    .expect_entity(*id, "officer camp soldier")
                    .camp()
                    == camp
            })
            .collect();
        let mut selected = head;
        if selected.is_none() {
            let mut minimum = 200 * 200;
            for &id in &registry {
                let Entity::Soldier(soldier) =
                    self.engine.expect_entity(id, "officer seeking soldier")
                else {
                    unreachable!()
                };
                if id == self.owner
                    || soldier.is_dead()
                    || soldier.is_unconscious()
                    || !soldier.element.active
                {
                    continue;
                }
                let brain = soldier.npc.ai_brain.enemy().expect("soldier brain");
                if brain.soldier_profile_rank != ProfileRank::Soldier
                    || !(brain.base.current_state == AiState::Default
                        || (brain.base.current_state == AiState::Seeking
                            && brain.base.current_substate == Substate::SeekingBodyReactiontime))
                {
                    continue;
                }
                let distance = self.square_distance(id, self.owner);
                if distance <= minimum {
                    minimum = distance;
                    selected = Some(id);
                }
            }
        }
        self.enemy_mut()
            .base
            .outbox
            .actor
            .delete_detectable_type(crate::element::DetectableType::Friend);
        self.settle();
        self.enemy_mut().base.alert_soldiers_point = self.enemy().base.seek_position;
        if reason != crate::ai::ReportType::Body {
            self.enemy_mut().base.detected_body = None;
        }
        if let Some(id) = head {
            self.enemy_mut()
                .base
                .outbox
                .actor
                .add_detectable((id, crate::element::DetectableType::Friend));
            self.settle();
        }
        for id in registry {
            if Some(id) == head {
                continue;
            }
            if self
                .engine
                .world
                .entities
                .expect_enemy_ai(id, format_args!("alert-list soldier rank"))
                .soldier_profile_rank
                != ProfileRank::Soldier
            {
                continue;
            }
            self.enemy_mut()
                .base
                .outbox
                .actor
                .add_detectable((id, crate::element::DetectableType::Friend));
            self.settle();
        }
        self.state(
            AiState::Seeking,
            Substate::SeekingOfficerLookingForSoldiers1,
        );
        if let Some(id) = selected {
            let target = self.engine.live_ai_position(id);
            let elevation = self
                .engine
                .expect_entity(id, "officer facing soldier")
                .position_iface()
                .get_elevation();
            self.engine.duty_face_position_at_elevation(
                self.sim,
                self.assets,
                self.owner,
                target,
                elevation,
            );
        } else {
            let direction = (self
                .engine
                .expect_entity(self.owner, "officer search direction")
                .element_data()
                .direction() as u16
                + 5)
                % 16;
            self.engine
                .duty_face_direction(self.sim, self.assets, self.owner, direction);
        }
        self.timer(20);
    }
    fn square_distance(&self, a: EntityId, b: EntityId) -> u32 {
        let a = self
            .engine
            .expect_entity(a, "alert distance actor")
            .element_data()
            .position();
        let b = self
            .engine
            .expect_entity(b, "alert distance actor")
            .element_data()
            .position();
        let dx = a.x - b.x;
        let dy = (a.y - b.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = a.z - b.z;
        (dx * dx + dy * dy + dz * dz) as u32
    }

    fn tower_guard_alert(&mut self, center: Position) {
        assert!(self.enemy().tower_guard);
        self.enemy_mut().base.seek_position = center;
        let hint = crate::ai::Hint {
            seek_point: center,
            seek_flags: 0,
            who_tells_me: AiEntityHandle::new(self.owner.index()),
        };
        self.engine.owner_work_speech(
            self.sim,
            self.assets,
            self.owner,
            crate::ai::AiSpeechAttempt {
                remark: crate::ai::Remark::CryAlert,
                flags: 0,
            },
        );
        let camp = self
            .engine
            .expect_entity(self.owner, "tower guard camp")
            .camp();
        let registry: Vec<_> = self
            .engine
            .ai
            .global
            .all_soldier_handles
            .iter()
            .copied()
            .map(|handle| EntityId::Soldier(crate::entity_id::SoldierId(handle)))
            .filter(|id| {
                self.engine
                    .expect_entity(*id, "tower guard camp member")
                    .camp()
                    == camp
            })
            .collect();
        let mut nearest = None;
        let mut far = None;
        let mut officer_distance = u32::MAX;
        let mut hearing_soldiers = 0usize;
        for &id in &registry {
            let distance = self.square_distance(id, self.owner);
            let Entity::Soldier(soldier) = self.engine.expect_entity(id, "tower guard recipient")
            else {
                unreachable!()
            };
            if id == self.owner
                || !soldier.is_able_to_help()
                || soldier
                    .npc
                    .ai_brain
                    .enemy()
                    .expect("soldier brain")
                    .tower_guard
                || self.engine.entity_data_in_building_sector(&soldier.element)
            {
                continue;
            }
            if distance < crate::ai_enemy::combat::SQR_TOWER_GUARD_ALERT_RADIUS as u32 {
                let mut stimulus = Stimulus::new(StimulusType::CallTowerGuardAlert);
                stimulus.info = StimulusInfo::Hint(hint);
                self.engine
                    .execute_ai_callback(self.sim, self.assets, id, &stimulus);
                match self
                    .engine
                    .world
                    .entities
                    .expect_enemy_ai(id, format_args!("alerted recipient rank"))
                    .soldier_profile_rank
                {
                    ProfileRank::Soldier if nearest.is_none() => hearing_soldiers += 1,
                    ProfileRank::Officer if distance < officer_distance => {
                        nearest = Some(id);
                        officer_distance = distance;
                    }
                    _ => {}
                }
            } else if self
                .engine
                .world
                .entities
                .expect_enemy_ai(id, format_args!("far officer rank"))
                .soldier_profile_rank
                == ProfileRank::Officer
                && distance < officer_distance
            {
                far = Some(id);
                officer_distance = distance;
            }
        }
        let recipient = nearest.or_else(|| {
            far.and_then(|officer| {
                let mut runner = None;
                for &id in registry.iter().take(hearing_soldiers) {
                    if self.square_distance(id, officer) < officer_distance {
                        runner = Some(id);
                    }
                }
                runner
            })
        });
        if let Some(recipient) = recipient {
            let mut stimulus = Stimulus::new(StimulusType::CallTowerGuardCallsMe);
            stimulus.info = StimulusInfo::Hint(hint);
            self.engine
                .execute_ai_callback(self.sim, self.assets, recipient, &stimulus);
        }
    }

    fn forecast(&self, target: EntityId) -> Position {
        let input = extract_exact_forecast_input(
            self.engine,
            self.engine
                .expect_entity(target, "officer destination forecast"),
            selected_actor_is_passing_door(&self.engine.orders.sequence_manager, target),
        )
        .expect("officer forecast requires an actor");
        crate::ai::prepare_forecast_destination_for_ia(
            &input,
            &self.engine.script_domains.interactables.doors,
            &self.engine.world.fast_grid.level.sectors,
            &self.engine.world.fast_grid.level.sector_number_map,
        )
        .resolve(self.sim)
        .position
    }

    fn alert_officer(&mut self) -> bool {
        use crate::ai::{AiEntityHandle, GotoFlags};
        use crate::ai_enemy::SeekFlags;
        assert_eq!(self.enemy().soldier_profile_rank, ProfileRank::Soldier);
        self.enemy_mut().base.outbox.actor.set_unfocus();
        self.settle();
        let camp = self
            .engine
            .expect_entity(self.owner, "officer search camp")
            .camp();
        let mut nearest = None;
        if self
            .enemy()
            .seek_flags
            .contains(SeekFlags::REPORT_OFFICER_AFTER)
            && let Some(antagonist) = self.enemy().base.antagonist
        {
            let target = self
                .engine
                .entity_id_for_index(antagonist.get())
                .expect("officer search antagonist is missing");
            match self
                .engine
                .world
                .entities
                .expect_ai_controller(target, format_args!("officer search antagonist"))
                .current_substate
            {
                Substate::SeekingOfficerWaitForInstructedSoldier => nearest = Some(target),
                Substate::SeekingOfficerWaitForInstructedGroup => {
                    self.state(AiState::Seeking, Substate::SeekingSoldierReturnToOfficer);
                    self.enemy_mut().base.set_emoticon(EmoticonType::None);
                    let target = self.engine.live_ai_position(target);
                    self.engine.duty_go_near(
                        self.sim,
                        self.assets,
                        self.owner,
                        target,
                        40,
                        GotoFlags::RUN,
                    );
                    self.timer(20);
                    self.enemy_mut()
                        .seek_flags
                        .remove(SeekFlags::REPORT_OFFICER_AFTER);
                    return true;
                }
                _ => {}
            }
        }
        if nearest.is_none() {
            let mut maximum = crate::ai_enemy::combat::MAX_ALERT_OFFICER_RADIUS as u32;
            let count = self.engine.ai.global.all_soldier_handles.len();
            for index in 0..count {
                let handle = self.engine.ai.global.all_soldier_handles[index];
                let id = EntityId::Soldier(crate::entity_id::SoldierId(handle));
                let Some(Entity::Soldier(soldier)) = self.engine.world.entities.get(id) else {
                    continue;
                };
                if soldier.soldier.cached_camp != camp {
                    continue;
                }
                let brain = soldier
                    .npc
                    .ai_brain
                    .enemy()
                    .expect("officer candidate requires brain");
                match brain.soldier_profile_rank {
                    ProfileRank::Officer => {
                        if !crate::element::Human::is_able_to_fight(soldier)
                            || brain.base.current_state != AiState::Default
                            || brain.base.ai_is_script_locked()
                        {
                            continue;
                        }
                        let other = soldier.element.position();
                        let mine = self
                            .engine
                            .expect_entity(self.owner, "officer search owner")
                            .element_data();
                        let me = mine.position();
                        let dx = (other.x - me.x).abs();
                        let dy = ((other.y - me.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO)
                            .abs();
                        let dz = (other.z - me.z).abs();
                        let mut distance = dx.max(dy).max(dz) as u32;
                        if self.engine.entity_data_in_building_sector(&soldier.element) {
                            distance += (crate::parameters_ai::LAYER_CHANGE_PENALTY
                                * (mine.layer() as f32 - soldier.element.layer() as f32).abs())
                                as u32;
                        }
                        if distance < maximum {
                            maximum = distance;
                            nearest = Some(id);
                        }
                    }
                    ProfileRank::Soldier
                        if officer_report_in_progress(brain.base.current_substate) =>
                    {
                        if self
                            .engine
                            .patrol_member_visible(self.assets, self.owner, id)
                        {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }
        let Some(officer) = nearest else {
            self.enemy_mut().seek_flags = SeekFlags::empty();
            return false;
        };
        self.enemy_mut().current_task_priority = task_priority::ALERT;
        self.state(AiState::Seeking, Substate::SeekingRunningToOfficer);
        self.enemy_mut().base.antagonist = Some(AiEntityHandle::new(officer.index()));
        self.enemy_mut()
            .base
            .outbox
            .actor
            .append_detectable((officer, crate::element::DetectableType::Friend));
        self.settle();
        let position = self.forecast(officer);
        self.enemy_mut().gather_position = position;
        self.engine.duty_go_near(
            self.sim,
            self.assets,
            self.owner,
            position,
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

    fn inside(&self) -> bool {
        self.engine.entity_data_in_building_sector(
            self.engine
                .expect_entity(self.owner, "officer building")
                .element_data(),
        )
    }

    fn alert_soldiers(&mut self, center: Position, flags: u16) -> bool {
        use crate::ai::Remark;
        use crate::ai_enemy::ReportUpdateFlags;
        use crate::ai_enemy::SeekFlags;
        let initial_position = self.engine.live_ai_position(self.owner);
        self.enemy_mut().base.seek_position = center;
        self.enemy_mut().seek_flags = SeekFlags::from_bits_truncate(flags);
        if self.enemy().seek_flags.contains(SeekFlags::DELAY) {
            self.state(AiState::Seeking, Substate::SeekingOfficerCallGroup);
            self.enemy_mut().base.set_emoticon(EmoticonType::XMark);
            self.timer(30);
            return true;
        }
        self.enemy_mut().base.outbox.actor.set_unfocus();
        self.settle();
        {
            let enemy = self.enemy_mut();
            enemy.current_task_priority = task_priority::ALERT;
            enemy.alerted_us.clear();
            enemy.base.list_alerted_us.clear();
            enemy.base.list_staying_us.clear();
            enemy.base.list_us.clear();
        }
        assert_eq!(self.enemy().soldier_profile_rank, ProfileRank::Officer);
        let camp = self.engine.expect_entity(self.owner, "alert camp").camp();
        let members = self.engine.ai.global.all_soldier_handles.clone();
        let mut average = crate::coordinates::MapVec::new(0.0, 0.0);
        for handle in members.iter().copied() {
            let target = EntityId::Soldier(crate::entity_id::SoldierId(handle));
            let Some(Entity::Soldier(soldier)) = self.engine.world.entities.get(target) else {
                continue;
            };
            if soldier.soldier.cached_camp != camp {
                continue;
            }
            let brain = soldier
                .npc
                .ai_brain
                .enemy()
                .expect("alert candidate requires a brain");
            if brain.soldier_profile_rank != ProfileRank::Soldier
                || !crate::element::Human::is_able_to_help(soldier)
            {
                continue;
            }
            let stays = i32::from(brain.base.blood_alcohol)
                > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
                || (soldier.element.active
                    && !self.engine.entity_data_in_building_sector(&soldier.element)
                    && (brain.tower_guard
                        || brain.soldier_profile_duty
                        || brain.company_number == 100));
            if stays && brain.base.patrol_chief != Some(self.owner) {
                continue;
            }
            if !self.engine.can_call_ai_soldier(self.owner, target) {
                continue;
            }
            let position = self.engine.live_ai_position(target);
            let dx = position.x - initial_position.x;
            let dy = position.y - initial_position.y;
            let radius = crate::ai_enemy::combat::ALERT_RADIUS as f32;
            if !(dx.abs().max(dy.abs()) < radius && dx * dx + dy * dy < radius * radius)
                || self.enemy().alerted_us.len() >= 20
            {
                continue;
            }
            self.engine
                .world
                .entities
                .expect_ai_controller_mut(target, format_args!("alert master"))
                .master = Some(AiEntityHandle::new(self.owner.index()));
            let mut stimulus = Stimulus::new(StimulusType::CallAlert);
            stimulus.info = StimulusInfo::Human(AiEntityHandle::new(self.owner.index()));
            if !self
                .engine
                .execute_ai_callback(self.sim, self.assets, target, &stimulus)
            {
                continue;
            }
            let target_world = self
                .engine
                .expect_entity(target, "accepted alert soldier")
                .element_data()
                .position();
            let owner_world = self
                .engine
                .expect_entity(self.owner, "accepted alert officer")
                .element_data()
                .position();
            let dx = target_world.x - owner_world.x;
            let dy =
                (target_world.y - owner_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_world.z - owner_world.z;
            let distance = dx * dx + dy * dy + dz * dz;
            self.engine
                .world
                .entities
                .expect_entity_mut(target, format_args!("alert sorting key"))
                .human_data_mut()
                .expect("soldier human data")
                .sorting_distance = distance;
            let insertion = self
                .enemy()
                .alerted_us
                .iter()
                .position(|prior| {
                    let prior = EntityId::Soldier(crate::entity_id::SoldierId(*prior));
                    !(distance
                        < self
                            .engine
                            .expect_entity(prior, "prior alert sorting key")
                            .human_data()
                            .expect("soldier human data")
                            .sorting_distance)
                })
                .unwrap_or(self.enemy().alerted_us.len());
            self.enemy_mut().alerted_us.insert(insertion, handle);
            self.engine.consider_live_ai_report(
                self.sim,
                self.assets,
                target,
                self.owner,
                ReportUpdateFlags::UPDATE_CHARLY.bits() | ReportUpdateFlags::UPDATE_TYPE.bits(),
            );
            if !self.inside() {
                let point = self.engine.live_ai_position(target);
                let owner = self.engine.live_ai_position(self.owner);
                let dx = point.x - owner.x;
                let dy = point.y - owner.y;
                let length = (dx * dx + dy * dy).sqrt();
                average.x += dx / length;
                average.y += dy / length;
            }
        }
        let door = if self.inside() {
            let Some(index) = self.enemy().base.my_door_index else {
                return false;
            };
            let door = self
                .engine
                .script_domains
                .interactables
                .doors
                .get(usize::from(index))
                .expect("stored officer exit door is missing");
            Some((
                door.point_out,
                door.point_mid,
                door.layer_out,
                crate::position_interface::SectorHandle::new(u16::from(door.sector_out)).map(
                    |handle| {
                        handle.with_arena_index(
                            door.sector_out_index
                                .expect("officer formation exit sector"),
                        )
                    },
                ),
            ))
        } else {
            None
        };
        if self.enemy().alerted_us.is_empty() {
            return false;
        }
        let count = self.enemy().alerted_us.len() as u16;
        let (direction_seed, mut try_point, step) = if let Some((out, mid, _, _)) = door {
            let dx = out.x - mid.x;
            let dy = out.y - mid.y;
            let norm = (dx * dx + (dy / crate::position_interface::ASPECT_RATIO).powi(2)).sqrt();
            (
                crate::position_interface::vector_to_sector_0_to_15_with_aspect(
                    dx,
                    dy,
                    crate::position_interface::ASPECT_RATIO,
                ) as u16,
                out,
                crate::coordinates::MapVec::new(dx / norm * 30.0, dy / norm * 30.0),
            )
        } else {
            (
                crate::position_interface::vector_to_sector_0_to_15_with_aspect(
                    average.x,
                    average.y,
                    crate::position_interface::ASPECT_RATIO,
                ) as u16,
                crate::coordinates::MapPoint::new(initial_position.x, initial_position.y),
                crate::coordinates::MapVec::new(0.0, 0.0),
            )
        };
        let mut selected = None;
        let mut direction = direction_seed;
        let mut future = initial_position;
        for attempt in 0..if door.is_some() { 10 } else { 1 } {
            let (layer, sector) = door.map_or(
                (initial_position.level, initial_position.sector),
                |(_, _, layer, sector)| (layer, sector),
            );
            if let Some((out, _, _, _)) = door {
                if attempt > 0
                    && !self.engine.world.fast_grid.is_straight_movement_authorized(
                        out,
                        try_point,
                        layer,
                        self.engine
                            .expect_entity(self.owner, "officer move box")
                            .element_data()
                            .sprite
                            .position_iface
                            .get_move_box(),
                    )
                {
                    break;
                }
            }
            for offset in 0..16 {
                direction = direction_seed + offset;
                if let Some(slots) =
                    self.formation_slots(try_point, direction & 15, count, layer, sector)
                {
                    selected = Some(slots);
                    break;
                }
            }
            // The officer's loop advances the test point once even on success.
            if door.is_some() {
                try_point.x += step.x;
                try_point.y += step.y;
            }
            if selected.is_some() {
                future = Position {
                    x: try_point.x,
                    y: try_point.y,
                    sector,
                    level: layer,
                };
                break;
            }
        }
        let placement = selected.is_some();
        if let Some(mut slots) = selected {
            for index in 0..usize::from(count) {
                let handle = self.enemy().alerted_us[index];
                let target = EntityId::Soldier(crate::entity_id::SoldierId(handle));
                let best = if self.inside() {
                    0
                } else {
                    let position = self.engine.live_ai_position(target);
                    let mut best = 0;
                    let mut minimum = u32::MAX as f32;
                    for (index, slot) in slots.iter().enumerate() {
                        let dx = position.x - slot.x;
                        let dy =
                            (position.y - slot.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                        let distance = dx * dx + dy * dy;
                        if distance < minimum {
                            minimum = distance;
                            best = index;
                        }
                    }
                    best
                };
                let position = slots.remove(best);
                let enemy = self
                    .engine
                    .world
                    .entities
                    .expect_enemy_ai_mut(target, format_args!("alert gather instruction"));
                enemy.gather_position = position;
                enemy.gather_direction = direction ^ 8;
                enemy.gather_position_instructed = true;
            }
        }
        if !self.inside() {
            use crate::element::Command;
            use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};
            self.enemy_mut().base.stop_all();
            self.settle();
            let mut sequence = Sequence::new();
            let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(self.owner));
            turn.set_property(
                Field::Direction,
                FieldValue::Integer(if placement {
                    direction
                } else {
                    (direction_seed + 16) ^ 8
                } as u32),
            );
            sequence.append_element(turn);
            sequence.append_element(SequenceElement::new(
                2,
                Command::GatherSoldiers,
                Some(self.owner),
            ));
            self.enemy_mut()
                .base
                .outbox
                .actor
                .launch_sequences
                .push(sequence);
            self.settle();
            self.engine.owner_work_speech(
                self.sim,
                self.assets,
                self.owner,
                crate::ai::AiSpeechAttempt {
                    remark: Remark::OfficerCallsGroup,
                    flags: 0,
                },
            );
            let frame = self.engine.control.frame_counter;
            self.enemy_mut()
                .base
                .set_transient_emoticon(EmoticonType::XMark, 20, frame);
            self.state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
            self.timer(20);
        } else if placement {
            self.enemy_mut().gather_position = future;
            self.enemy_mut().gather_direction = direction;
            self.state(
                AiState::Seeking,
                Substate::SeekingOfficerWaitInsideHouseToInstructGroup,
            );
            self.timer(50);
        } else {
            self.state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
            self.timer(20);
        }
        true
    }

    fn formation_slots(
        &self,
        point: crate::coordinates::MapPoint,
        direction: u16,
        count: u16,
        layer: u16,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<Vec<Position>> {
        let mut width = crate::ai_enemy::combat::STANDARD_LINE_LENGTH.max(1) as u16;
        if count > 1 {
            while count % width == 1 {
                width += 1;
            }
        }
        let forward = crate::position_interface::sector_to_vector_iso(direction as i16);
        let side = crate::position_interface::sector_to_vector_iso(((direction + 4) % 16) as i16);
        let move_box = self
            .engine
            .expect_entity(self.owner, "formation move box")
            .element_data()
            .sprite
            .position_iface
            .get_move_box();
        let mut slots = Vec::with_capacity(count as usize);
        for index in 0..count {
            let backwards = (index / width) as f32;
            let rest = index % width;
            let sideways = if rest & 1 == 1 {
                rest.div_ceil(2) as f32
            } else {
                -((rest / 2) as f32)
            };
            let x = point.x
                + forward[0] * 50.0
                + sideways * (side[0] * 50.0)
                + backwards * (forward[0] * 30.0);
            let y = point.y
                + forward[1] * 50.0
                + sideways * (side[1] * 50.0)
                + backwards * (forward[1] * 30.0);
            if !self.engine.world.fast_grid.is_straight_movement_authorized(
                point,
                crate::coordinates::MapPoint::new(x, y),
                layer,
                move_box,
            ) {
                return None;
            }
            slots.push(Position {
                x,
                y,
                sector,
                level: layer,
            });
        }
        Some(slots)
    }

    fn enemy(&self) -> &EnemyAi {
        self.engine
            .world
            .entities
            .expect_enemy_ai(self.owner, format_args!("officer coordination"))
    }

    fn enemy_mut(&mut self) -> &mut EnemyAi {
        self.engine
            .world
            .entities
            .expect_enemy_ai_mut(self.owner, format_args!("officer coordination"))
    }

    fn settle(&mut self) {
        self.engine
            .drain_direct_ai_owner_boundary(self.sim, self.owner, self.assets);
    }

    fn state(&mut self, state: AiState, substate: Substate) {
        self.engine
            .duty_set_state(self.sim, self.assets, self.owner, state, substate);
    }

    fn timer(&mut self, frames: u32) {
        let frame = self.engine.control.frame_counter;
        self.enemy_mut().base.launch_timer(frames, frame);
    }

    fn command_soldiers_to_attack(&mut self, center: Position) -> bool {
        assert_eq!(self.enemy().soldier_profile_rank, ProfileRank::Officer);
        let initial_position = self.engine.live_ai_position(self.owner);
        self.enemy_mut().base.seek_position = center;
        self.enemy_mut().current_task_priority = task_priority::ALERT;
        let mut accepted = 0_u16;
        let mut average = crate::coordinates::MapVec::new(0.0, 0.0);
        // Retain only registration identities across callbacks. Eligibility,
        // visibility, and positions are queried at each member's call site.
        let members: Vec<_> = self.engine.world.entities.npc_ids().collect();
        for member in members {
            let entity = self.engine.expect_entity(member, "combat alert recipient");
            let Entity::Soldier(soldier) = entity else {
                continue;
            };
            if soldier
                .npc
                .ai_brain
                .enemy()
                .expect("soldier has no hostile brain")
                .soldier_profile_rank
                != ProfileRank::Soldier
                || !crate::element::Human::is_able_to_fight(soldier)
                || !self
                    .engine
                    .patrol_member_visible(self.assets, member, self.owner)
            {
                continue;
            }
            let position = self.engine.live_ai_position(member);
            let dx = position.x - initial_position.x;
            let dy = position.y - initial_position.y;
            let radius = crate::ai_enemy::combat::ALERT_RADIUS as f32;
            if !(dx.abs().max(dy.abs()) < radius && dx * dx + dy * dy < radius * radius) {
                continue;
            }
            let stimulus = Stimulus::with_position(StimulusType::CallCombatAlert, center);
            if self
                .engine
                .execute_ai_callback(self.sim, self.assets, member, &stimulus)
            {
                accepted += 1;
                let member_position = self.engine.live_ai_position(member);
                let owner_position = self.engine.live_ai_position(self.owner);
                let dx = member_position.x - owner_position.x;
                let dy = member_position.y - owner_position.y;
                let length = (dx * dx + dy * dy).sqrt();
                average.x += dx / length;
                average.y += dy / length;
            }
        }
        if accepted == 0 {
            return false;
        }

        let seek = self.enemy().base.seek_position;
        let target_world =
            self.engine
                .position_to_point_3d(self.assets, seek.sector, seek.level, seek.x, seek.y);
        let owner_world = self
            .engine
            .expect_entity(self.owner, "combat alert pointing")
            .element_data()
            .position();
        let point_direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target_world.x - owner_world.x,
            target_world.y - owner_world.y,
        ) as u16;
        self.enemy_mut().base.stop_all();
        self.settle();
        use crate::element::Command;
        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};
        let mut sequence = Sequence::new();
        let mut level = 1;
        let now = self.engine.live_ai_position(self.owner);
        if (center.x - now.x).abs().max((center.y - now.y).abs()) > 150.0 {
            let direction = crate::position_interface::vector_to_sector_0_to_15_with_aspect(
                average.x,
                average.y,
                crate::position_interface::ASPECT_RATIO,
            ) as u16;
            let mut turn = SequenceElement::new_generic(level, Command::Turn, Some(self.owner));
            turn.set_property(Field::Direction, FieldValue::Integer(direction as u32));
            sequence.append_element(turn);
            level += 1;
            sequence.append_element(SequenceElement::new(
                level,
                Command::GatherSoldiers,
                Some(self.owner),
            ));
            level += 1;
        }
        let mut point = SequenceElement::new_generic(level, Command::Point, Some(self.owner));
        point.set_property(
            Field::Direction,
            FieldValue::Integer(point_direction as u32),
        );
        sequence.append_element(point);
        self.enemy_mut()
            .base
            .outbox
            .actor
            .launch_sequences
            .push(sequence);
        self.settle();
        let frame = self.engine.control.frame_counter;
        self.enemy_mut()
            .base
            .set_transient_emoticon(EmoticonType::XMark, 20, frame);
        self.state(AiState::Attacking, Substate::AttackingOfficerGivingOrders);
        self.timer(20);
        self.enemy_mut().base.friends_are_alerted = true;
        true
    }
}
