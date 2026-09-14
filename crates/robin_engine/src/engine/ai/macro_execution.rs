//! Synchronous waypoint bytecode execution with short actor borrows.

use super::*;
use crate::ai::*;
use crate::sim_rng::SimulationContext;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct MacroOwner {
    frame: u32,
    original_creation_order: Option<u32>,
    self_is_soldier: bool,
    self_rank: crate::profiles::ProfileRank,
}

struct MacroExecution<'a> {
    engine: &'a mut EngineInner,
    owner: EntityId,
    sim: &'a crate::sim_rng::SimulationContext,
    assets: &'a LevelAssets,
}

impl std::ops::Deref for MacroExecution<'_> {
    type Target = AiController;
    fn deref(&self) -> &Self::Target {
        self.engine
            .world
            .entities
            .expect_ai_controller(self.owner, format_args!("macro owner"))
    }
}

impl std::ops::DerefMut for MacroExecution<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.engine
            .world
            .entities
            .expect_ai_controller_mut(self.owner, format_args!("macro owner"))
    }
}

impl EngineInner {
    pub(in crate::engine) fn execute_ai_assign_post(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        post_position: Position,
        post_direction: u16,
    ) {
        self.execute_ai_break_macro(owner);
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("post assignment"));
        ai.path_id = None;
        ai.detach_patrol_path(None, false);
        ai.has_patrol_path = false;
        ai.initial_position = post_position;
        ai.initial_view_direction = post_direction & 0x0F;
        ai.is_stay_at_home = false;
        ai.likes_to_sit_around = false;
        ai.special_action = false;

        if !ai.script_locked && ai.current_state == AiState::Default {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReturnToDuty),
            );
        }
    }

    pub(in crate::engine) fn execute_ai_assign_patrol_path(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        assignment: PatrolAssignment,
        script_way: bool,
    ) -> bool {
        self.execute_ai_break_macro(owner);
        let current_position = self.live_ai_position(owner);
        let current_direction = self
            .expect_entity(owner, "path assignment owner")
            .element_data()
            .direction() as u16;
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("path assignment"));
        match assignment {
            PatrolAssignment::ClearPath | PatrolAssignment::ClearPathSitAround => {
                let sits = matches!(assignment, PatrolAssignment::ClearPathSitAround);
                ai.has_patrol_path = false;
                ai.detach_patrol_path(None, false);
                ai.path_id = None;
                ai.initial_position = current_position;
                ai.initial_view_direction = current_direction & 0x0F;
                if !script_way {
                    ai.likes_to_sit_around = sits;
                    ai.special_action = false;
                }
                ai.is_stay_at_home = false;
                if !ai.script_locked && ai.current_state == AiState::Default {
                    self.execute_ai_callback(
                        sim,
                        assets,
                        owner,
                        &Stimulus::new(StimulusType::EventReturnToDuty),
                    );
                }
                if script_way {
                    let ai = self.world.entities.expect_ai_controller_mut(
                        owner,
                        format_args!("path assignment callback return"),
                    );
                    ai.likes_to_sit_around = sits;
                    ai.special_action = false;
                }
                true
            }
            PatrolAssignment::Index(pid) | PatrolAssignment::ScriptWay(pid) => {
                let idx = pid.get() as usize;
                // The original game writes the patrol-path flag before validating the
                // authored index. Preserve that odd partial mutation on the
                // error path; mPath itself is not reinitialized there.
                ai.has_patrol_path = true;
                // Strictly greater, so `idx == count` is tolerated
                // (matches the off-by-one in the original engine).
                //
                // todo: the hiking-path overload has no bounds check at
                // all — the script hands it an already-resolved pointer. Rust
                // resolves the script's Way to an index here, so `ScriptWay`
                // inherits the index overload's guard. Keep it (failing loud
                // beats a wild dereference), but note the divergence.
                if idx > assets.navigation.hiking_paths.len() {
                    tracing::warn!(
                        npc = ai.me,
                        idx = pid.get(),
                        count = assets.navigation.hiking_paths.len(),
                        "patrol-path assignment: index out of range",
                    );
                    return false;
                }
                ai.path_id = Some(pid);
                let (last_waypoint_index, history) = if let Some(path) = ai.patrol_path.take() {
                    (path.last_waypoint_index, path.history)
                } else {
                    (
                        ai.detached_patrol_path_status.last_waypoint_index,
                        std::mem::take(&mut ai.detached_patrol_path_status.history),
                    )
                };
                ai.patrol_path =
                    PatrolPath::new(pid, &assets.navigation.hiking_paths).map(|mut path| {
                        // Initializing a new path resets current/forward only.
                        path.last_waypoint_index = last_waypoint_index;
                        path.history = history;
                        path
                    });
                ai.likes_to_sit_around = false;
                // Only the waypoint-macro index overload
                // (patrol-path assignment by 16-bit index,
                // clears
                // special-action flag. The `AssignPath` script native goes
                // through the hiking-path overload
                // whose valid-path arm
                // leaves that special-action flag untouched — a leisure-authored NPC
                // sent onto a scripted route stays special, so its later
                // return-to-duty movement skips the already-on-point shortcut
                // and runs a real (possibly zero-length) move instead.
                if matches!(assignment, PatrolAssignment::Index(_)) {
                    ai.special_action = false;
                }
                if !ai.script_locked && ai.current_state == AiState::Default {
                    self.execute_ai_callback(
                        sim,
                        assets,
                        owner,
                        &Stimulus::new(StimulusType::EventReturnToDuty),
                    );
                }
                true
            }
        }
    }

    pub(in crate::engine) fn execute_ai_script_unlock(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.execute_ai_blink_all_enemies(owner);
        let unconscious = self
            .expect_entity(owner, "script unlock")
            .human_data()
            .expect("script unlock human")
            .unconscious;
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("script unlock"));
        let after_script = ai
            .stimulus_queue
            .iter()
            .any(|s| s.stimulus_type == StimulusType::EventAfterScriptGoOn);
        ai.script_locked = false;
        if ai.current_state != AiState::Sleeping && !after_script && !unconscious {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReturnToDuty),
            );
        }
    }

    pub(in crate::engine) fn run_ai_macro(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        MacroExecution {
            engine: self,
            owner,
            sim,
            assets,
        }
        .run();
    }
}

impl MacroExecution<'_> {
    fn owner_state(&self) -> MacroOwner {
        let entity = self.engine.expect_entity(self.owner, "macro owner state");
        MacroOwner {
            frame: self.engine.control.frame_counter,
            original_creation_order: Some(self.engine.world.original_creation_order(self.owner)),
            self_is_soldier: entity.enemy_ai().is_some(),
            self_rank: entity
                .enemy_ai()
                .map_or(crate::profiles::ProfileRank::None, |ai| {
                    ai.soldier_profile_rank
                }),
        }
    }

    fn debug_macro_lifecycle(&self, owner: &MacroOwner, phase: &str, reason: impl std::fmt::Debug) {
        self.debug_macro_lifecycle_at(owner.frame, owner.original_creation_order, phase, reason);
    }

    fn break_macro_debug(&mut self, owner: &MacroOwner, reason: &str) {
        self.debug_macro_lifecycle(owner, "break_before", reason);
        self.engine.execute_ai_break_macro(self.owner);
        self.debug_macro_lifecycle(owner, "break_after", reason);
    }

    fn finish_patrol_macro_debug(&mut self, owner: &MacroOwner, reason: &str) {
        self.debug_macro_lifecycle(owner, "finish_before", reason);
        self.finish_patrol_macro();
        self.debug_macro_lifecycle(owner, "finish_after", reason);
    }

    fn run(&mut self) {
        self.execute_next_macro_command(self.sim);
    }

    fn set_macro_state(&mut self, substate: Substate) {
        self.engine.duty_set_state(
            self.sim,
            self.assets,
            self.owner,
            AiState::Default,
            substate,
        );
    }

    fn callback(&mut self, event: StimulusType) {
        self.engine
            .execute_ai_callback(self.sim, self.assets, self.owner, &Stimulus::new(event));
    }

    fn speak(&mut self, remark: Remark) {
        self.engine.execute_ai_speech(
            self.sim,
            self.assets,
            self.owner,
            AiSpeechAttempt {
                remark,
                flags: SpeechFlags::empty().bits(),
            },
        );
    }

    fn consume_macro_operand(&mut self) {
        self.macro_command_offset += 2;
        self.number_of_remaining_macro_bytes =
            self.number_of_remaining_macro_bytes.saturating_sub(2);
    }

    fn assign_path(&mut self, assignment: PatrolAssignment) {
        self.engine.execute_ai_assign_patrol_path(
            self.sim,
            self.assets,
            self.owner,
            assignment,
            false,
        );
    }

    fn execute_next_macro_command(&mut self, sim: &crate::sim_rng::SimulationContext) {
        let mut point_already_set = false;
        'vm: loop {
            let entry_ctx = self.owner_state();
            self.debug_macro_lifecycle(&entry_ctx, "execute_enter", "execute_next_macro_command");
            // Loop iterations retain recursive entry semantics: even a repeated
            // civilian substate can synchronously notify its script.
            if self.current_state == AiState::Default {
                self.set_macro_state(Substate::DefaultInMacro);
            }
            self.standing_around_timer = 0;
            let ctx = &self.owner_state();
            if (self.number_of_remaining_macro_bytes as i16) > 0 {
                let opcode_byte = match self.macro_command.get(self.macro_command_offset).copied() {
                    Some(b) => b,
                    None => {
                        tracing::warn!(
                            "NPC {}: macro PC out of bounds at offset {}",
                            self.me,
                            self.macro_command_offset
                        );
                        self.break_macro_debug(ctx, "macro_pc_out_of_bounds");
                        return;
                    }
                };
                self.macro_command_offset += 1;
                self.number_of_remaining_macro_bytes -= 1;
                self.macro_in_progress = true;

                let Some(opcode) = MacroOpcode::from_u8(opcode_byte) else {
                    tracing::warn!(
                        "NPC {}: invalid macro opcode 0x{:02x}, breaking macro",
                        self.me,
                        opcode_byte
                    );
                    self.number_of_remaining_macro_bytes = 0;
                    continue 'vm;
                };
                self.debug_macro_lifecycle(ctx, "opcode_started", opcode);

                match self.execute_macro_opcode(opcode, &mut point_already_set, sim, ctx) {
                    std::ops::ControlFlow::Continue(()) => continue 'vm,
                    std::ops::ControlFlow::Break(()) => return,
                }
            } else {
                let path_size = self.patrol_path.as_ref().map(|p| p.size).unwrap_or(0);

                if path_size == 1 {
                    if self.macro_started_in_this_frame {
                        self.set_macro_state(Substate::DefaultInMacro);
                        self.macro_started_in_this_frame = false;
                        self.launch_macro_timer(
                            crate::parameters_ai::AI_ONE_POINT_DEFAULT_TIME as u32,
                            ctx.frame,
                        );
                        self.debug_macro_lifecycle(ctx, "timer_started", "one_point_path");
                    } else {
                        self.finish_patrol_macro_debug(ctx, "one_point_reach_point");
                        self.set_macro_state(Substate::DefaultEnroute);
                        self.callback(StimulusType::EventReachPoint);
                    }
                } else {
                    if !point_already_set && let Some(ref mut path) = self.patrol_path {
                        path.advance();
                    }

                    self.set_macro_state(Substate::DefaultEnroute);
                    let ctx = &self.owner_state();
                    let assets = self.assets;
                    let hiking_paths = &assets.navigation.hiking_paths;
                    let will_stop = self.will_stop_at_next_waypoint_at(
                        sim,
                        hiking_paths,
                        ctx.frame,
                        ctx.original_creation_order,
                        WillStopCaller::MacroCompletion,
                    );
                    let mut walk_flags = self.default_path_walking_flags;
                    if !will_stop {
                        walk_flags |= GotoFlags::DONT_STOP;
                    }
                    if let Some(next_wp) = self
                        .patrol_path
                        .as_ref()
                        .and_then(|p| {
                            p.current_waypoint(hiking_paths)
                                .map(|wp| (p.hiking_path_index, p.current_waypoint_index, wp))
                        })
                        .map(|(path_index, waypoint_index, wp)| Position {
                            x: wp.x as f32,
                            y: wp.y as f32,
                            sector: assets.navigation.hiking_waypoint_sector(
                                usize::from(path_index),
                                usize::from(waypoint_index),
                                wp.sector,
                            ),
                            level: wp.level,
                        })
                    {
                        self.engine
                            .duty_go_to(sim, self.assets, self.owner, next_wp, walk_flags);
                        // An already-reached waypoint can start another macro.
                        // Its deadline survives this invocation's cancellation.

                        self.finish_patrol_macro_debug(ctx, "goto_completed");
                    } else {
                        self.engine.execute_ai_return_to_duty(
                            sim,
                            self.assets,
                            self.owner,
                            DutyFlags::empty(),
                        );
                        self.finish_patrol_macro_debug(ctx, "missing_next_waypoint");
                    }
                }
                return;
            }
        }
    }
}

impl MacroExecution<'_> {
    fn execute_macro_opcode(
        &mut self,
        opcode: MacroOpcode,
        point_already_set: &mut bool,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &MacroOwner,
    ) -> std::ops::ControlFlow<()> {
        match opcode {
            MacroOpcode::ReversePath => {
                if let Some(ref mut path) = self.patrol_path {
                    path.flip_forward_movement();
                }
                return std::ops::ControlFlow::Continue(());
            }

            MacroOpcode::SkipPoint => {
                if let Some(ref mut path) = self.patrol_path {
                    path.advance();
                }
                self.number_of_remaining_macro_bytes = 0;
                return std::ops::ControlFlow::Continue(());
            }

            MacroOpcode::GotoPoint => {
                let Some(index) = self.peek_macro_u16() else {
                    self.break_macro_debug(ctx, "goto_point_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                let owner = self.me;
                if let Some(ref mut path) = self.patrol_path {
                    if path.current_waypoint_index as u16 == index {
                        tracing::warn!("NPC {}: CMD_GOTO_POINT → same waypoint {}", owner, index);
                    }
                    path.set_current_index(index as u8);
                }
                self.number_of_remaining_macro_bytes = 0;
                *point_already_set = true;
                return std::ops::ControlFlow::Continue(());
            }

            MacroOpcode::FaceTo => {
                self.set_macro_state(Substate::DefaultInMacroWaitingForDone);
                let Some(direction) = self.peek_macro_u16() else {
                    self.break_macro_debug(ctx, "face_to_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                self.engine
                    .duty_face_direction(sim, self.assets, self.owner, direction);

                self.consume_macro_operand();
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Wait => {
                let Some(frames) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "wait_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                self.launch_macro_timer(frames as u32, ctx.frame);
                self.debug_macro_lifecycle(ctx, "timer_started", "wait");
                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Check4 => {
                let Some(friend_id) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "check4_friend_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                let Some(frames) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "check4_frames_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_CHECK_4 is illegal for civilians", self.me);
                }
                self.engine.initialize_ai_friend_check(
                    sim,
                    self.assets,
                    self.owner,
                    friend_id,
                    frames,
                    u16::MAX,
                );

                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Check4Sync => {
                let Some(friend_id) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "check4_sync_friend_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                let Some(frames) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "check4_sync_frames_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                let Some(index) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "check4_sync_index_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_CHECK_4_SYNC is illegal for civilians", self.me);
                }
                self.engine.initialize_ai_friend_check(
                    sim,
                    self.assets,
                    self.owner,
                    friend_id,
                    frames,
                    index,
                );

                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::StayHere => {
                self.assign_path(PatrolAssignment::ClearPath);
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::ChangeWay => {
                let Some(index) = self.peek_macro_u16() else {
                    self.break_macro_debug(ctx, "change_way_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                let assignment = match PathId::new(index) {
                    Some(pid) => PatrolAssignment::Index(pid),
                    None => PatrolAssignment::ClearPath,
                };
                self.assign_path(assignment);
                // Assignment's nested decision finishes before this explicit
                // second cancellation and actor-specific duty call.
                self.engine.execute_ai_break_macro(self.owner);
                self.engine.execute_ai_return_to_duty(
                    sim,
                    self.assets,
                    self.owner,
                    DutyFlags::empty(),
                );
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Run => {
                self.default_path_walking_flags |= GotoFlags::RUN;
                // Movement records the raw flags before civilian sanitation.
                self.run();
                if !ctx.self_is_soldier
                    && self
                        .default_path_walking_flags
                        .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                {
                    tracing::warn!(
                        me = self.me,
                        "civilian CMD_RUN with forbidden movement flags — masking",
                    );
                    self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                }
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Walk => {
                self.default_path_walking_flags -= GotoFlags::RUN;
                self.run();
                if !ctx.self_is_soldier
                    && self
                        .default_path_walking_flags
                        .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                {
                    tracing::warn!(
                        me = self.me,
                        "civilian CMD_WALK with forbidden movement flags — masking",
                    );
                    self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                }
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::LookLeft => {
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_LOOK_LEFT is illegal for civilians", self.me);
                }
                self.engine
                    .execute_ai_look_sidewards(self.owner, LookDirection::Left);

                self.set_macro_state(Substate::DefaultInMacroWaitingForDone);
                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::LookRight => {
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_LOOK_RIGHT is illegal for civilians", self.me);
                }
                self.engine
                    .execute_ai_look_sidewards(self.owner, LookDirection::Right);

                self.set_macro_state(Substate::DefaultInMacroWaitingForDone);
                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::Bend => {
                let Some(frames) = self.read_macro_u16() else {
                    self.break_macro_debug(ctx, "bend_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_BEND is illegal for civilians", self.me);
                }
                self.engine
                    .execute_ai_look_sidewards(self.owner, LookDirection::Down);
                self.launch_macro_timer(frames as u32, ctx.frame);
                self.debug_macro_lifecycle(ctx, "timer_started", "bend");
                self.macro_started_in_this_frame = false;
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::PatrolStop => {
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_PATROL_STOP is illegal for civilians", self.me);
                }
                self.patrol_stopped = true;
                if ctx.self_is_soldier && ctx.self_rank == crate::profiles::ProfileRank::Officer {
                    self.speak(Remark::OfficerStopsPatrol);
                }
                self.run();
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::PatrolDirection => {
                let Some(direction) = self.peek_macro_u16() else {
                    self.break_macro_debug(ctx, "patrol_direction_truncated");
                    return std::ops::ControlFlow::Break(());
                };
                if !ctx.self_is_soldier {
                    tracing::warn!(
                        "NPC {}: CMD_PATROL_DIRECTION is illegal for civilians",
                        self.me
                    );
                }
                self.engine.instruct_patrol_direction_to_patrol_members(
                    sim,
                    self.owner,
                    self.assets,
                    direction,
                );
                self.consume_macro_operand();
                self.run();
                return std::ops::ControlFlow::Break(());
            }

            MacroOpcode::PatrolStart => {
                if !ctx.self_is_soldier {
                    tracing::warn!("NPC {}: CMD_PATROL_START is illegal for civilians", self.me);
                }
                self.patrol_stopped = false;
                if ctx.self_is_soldier && ctx.self_rank == crate::profiles::ProfileRank::Officer {
                    self.speak(Remark::OfficerStartsPatrol);
                }
                self.engine
                    .initialize_patrol_for_npc(self.assets, self.owner);
                self.run();
                return std::ops::ControlFlow::Break(());
            }
        }
    }
}

#[cfg(test)]
mod assignment_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::test_support::{actors::make_test_civilian, square_sector};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    fn macro_owner(
        command: Vec<u8>,
        walking_flags: GotoFlags,
    ) -> (EngineInner, LevelAssets, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(64, 64);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_index = engine.world.fast_grid_mut().add_sector(
            square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(1000.0, 1000.0)),
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(sector_index).unwrap());
        let mut entity = make_test_civilian(crate::element::Posture::Upright);
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(10.0, 10.0));
        entity.element_data_mut().set_sector(Some(sector));
        entity.npc_data_mut().unwrap().life_points = 100;
        entity.npc_data_mut().unwrap().ai_brain =
            crate::element::AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(0)));
        let owner = engine.add_test_entity(entity);
        let paths = vec![RawHikingPath {
            waypoints: [10, 30, 50]
                .into_iter()
                .map(|x| RawWaypoint {
                    x,
                    y: 10,
                    sector: 1,
                    level: 0,
                    command: WaypointCommand::None,
                })
                .collect(),
        }];
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        assets.navigation.hiking_paths = std::sync::Arc::new(paths);
        assets.navigation.hiking_waypoint_sectors =
            Some(std::sync::Arc::new(vec![vec![sector; 3]]));
        let ai = engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("test macro owner"));
        ai.current_state = AiState::Default;
        ai.current_substate = Substate::DefaultInMacro;
        ai.has_patrol_path = true;
        ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &assets.navigation.hiking_paths);
        ai.number_of_remaining_macro_bytes = command.len() as u16;
        ai.macro_command = command;
        ai.default_path_walking_flags = walking_flags;
        (engine, assets, owner)
    }

    #[test]
    fn goto_point_keeps_the_unconsumed_operand_cursor() {
        let (mut engine, assets, owner) =
            macro_owner(vec![MacroOpcode::GotoPoint as u8, 2, 0], GotoFlags::empty());
        engine.run_ai_macro(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("completed macro"));
        assert_eq!(ai.patrol_path.as_ref().unwrap().current_waypoint_index, 2);
        assert_eq!(ai.macro_command_offset, 1);
    }

    #[test]
    fn civilian_run_sanitizes_flags_after_nested_path_completion() {
        let (mut engine, assets, owner) =
            macro_owner(vec![MacroOpcode::Run as u8], GotoFlags::BACK);
        engine.run_ai_macro(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("completed macro"));
        assert!(ai.last_goto_flags.contains(GotoFlags::RUN));
        assert!(ai.last_goto_flags.contains(GotoFlags::BACK));
        assert_eq!(ai.default_path_walking_flags, GotoFlags::RUN);
    }
}
