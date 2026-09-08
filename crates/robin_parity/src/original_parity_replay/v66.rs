//! Frozen v66 wire layouts and one-way compatibility conversion.
//!
//! Field and variant order, field types, and shared child layouts are part of
//! authoritative bitcode artifacts. Do not modernize these shapes. Conversion
//! may reconstruct only the omissions documented by this generation. Reverse
//! conversions are test-only: production always writes the current layout.
use super::*;

/// Header layout written by native trace version 66. This is the oldest
/// authoritative native format in the retained corpus and must remain frozen.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceHeaderV66 {
    pub(super) record_type: String,
    pub(super) mission: String,
    pub(super) proto_level: String,
    pub(super) rng_seed: u64,
    pub(super) schema: u32,
    pub(super) session_index: u32,
    pub(super) start_state: TraceStartState,
    pub(super) initial_frame: u64,
    pub(super) simulation_hz: u32,
    pub(super) synchronous_pathfinding: bool,
    pub(super) rng_stream: String,
    pub(super) visibility_queries: String,
    pub(super) authoritative_state: Option<String>,
    pub(super) random_input_seed: Option<u32>,
    pub(super) sim_config: TraceSimConfig,
    pub(super) campaign: TraceCampaign,
    pub(super) motion_grid: TraceMotionGrid,
    pub(super) initial_npc_transients: Option<Vec<TraceInitialNpcTransient>>,
    pub(super) initial_save: Option<TraceInitialSave>,
}

impl From<TraceHeaderV66> for TraceHeader {
    fn from(header: TraceHeaderV66) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV66 {
    fn from(header: TraceHeader) -> Self {
        Self {
            record_type: header.record_type,
            mission: header.mission,
            proto_level: header.proto_level,
            rng_seed: header.rng_seed,
            schema: header.schema,
            session_index: header.session_index,
            start_state: header.start_state,
            initial_frame: header.initial_frame,
            simulation_hz: header.simulation_hz,
            synchronous_pathfinding: header.synchronous_pathfinding,
            rng_stream: header.rng_stream,
            visibility_queries: header.visibility_queries,
            authoritative_state: None,
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients,
            initial_save: header.initial_save,
        }
    }
}

/// Command layout embedded in native trace version 66. In particular,
/// `SwordStrike::seek_distance` was optional on disk.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum TraceCommandV66 {
    BoxSelect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    GroupMove {
        actors: Vec<TraceEntityId>,
        destination: TracePoint,
        running: bool,
        show_marker: bool,
        goal_sector: i16,
        goal_layer: u16,
    },
    LaunchInteraction {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        running: bool,
    },
    LaunchSelfAbility {
        actor: TraceEntityId,
        original_command: u32,
        original_command_name: String,
    },
    LaunchGroundTarget {
        actor: TraceEntityId,
        target: TracePoint3,
        original_command: u32,
        original_command_name: String,
        original_target_field: u32,
        titbit_layer: u16,
    },
    LaunchScrollRead {
        actor: TraceEntityId,
        target: TraceEntityId,
        running: bool,
    },
    SwordStrike {
        actor: TraceEntityId,
        target: TraceEntityId,
        original_command: u32,
        original_command_name: String,
        with_seek: bool,
        seek_distance: Option<f32>,
    },
    SelectPc {
        pc: TraceEntityId,
        append: bool,
    },
    UnselectAllPcs,
    StopPc {
        pc: TraceEntityId,
    },
    SelectAction {
        pc: TraceEntityId,
        action: TraceAction,
        original_action: u32,
    },
    CancelAction {
        pc: Option<TraceEntityId>,
        action: TraceAction,
        original_action: u32,
    },
    OrientActionAt {
        action: TraceAction,
        original_action: u32,
        actor: TraceEntityId,
        mouse_map: TracePoint,
        target: TracePoint3,
    },
    MakePcFast {
        entity: TraceEntityId,
    },
    CrouchDown,
    StandUp,
    DropAleAt {
        actor: TraceEntityId,
        target: TracePoint,
        running: bool,
    },
    ShieldSelectProtected {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
    },
    BoxUnselect {
        first: TracePoint,
        second: TracePoint,
        append: bool,
    },
    RaiseShieldWithDanger {
        actor: TraceEntityId,
        protected_pc: TraceEntityId,
        danger_point: TracePoint3,
        danger_point_layer: u16,
    },
    TeleportSelected {
        destination: TracePoint,
        goal_sector: i16,
        goal_layer: u16,
    },
    SelectAllPcs,
    UnselectPc {
        pc: TraceEntityId,
    },
    SelectActionIndex {
        index: u32,
    },
    SetLockAlt {
        on: bool,
    },
    KeyControl,
    KeyReleaseControl,
    StartMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    DeleteMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    StartRecordingMacro {
        pc: Option<TraceEntityId>,
        slot: u8,
    },
    ChangeQaMemory {
        slot: u8,
    },
    HeroRefusedAction {
        actor: TraceEntityId,
        action: TraceAction,
        original_action: u32,
        target: Option<TraceEntityId>,
        reason: String,
    },
    BeggarDontTalkStamp {
        entity: TraceEntityId,
    },
}

impl TraceCommandV66 {
    pub(super) fn into_current(self) -> TraceCommand {
        match self {
            Self::BoxSelect {
                first,
                second,
                append,
            } => TraceCommand::BoxSelect {
                first,
                second,
                append,
            },
            Self::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            } => TraceCommand::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            },
            Self::LaunchInteraction {
                actor,
                target,
                original_command,
                original_command_name,
                running,
            } => TraceCommand::LaunchInteraction {
                actor,
                target,
                original_command,
                original_command_name,
                running,
            },
            Self::LaunchSelfAbility {
                actor,
                original_command,
                original_command_name,
            } => TraceCommand::LaunchSelfAbility {
                actor,
                original_command,
                original_command_name,
            },
            Self::LaunchGroundTarget {
                actor,
                target,
                original_command,
                original_command_name,
                original_target_field,
                titbit_layer,
            } => TraceCommand::LaunchGroundTarget {
                actor,
                target,
                original_command,
                original_command_name,
                original_target_field,
                titbit_layer,
            },
            Self::LaunchScrollRead {
                actor,
                target,
                running,
            } => TraceCommand::LaunchScrollRead {
                actor,
                target,
                running,
            },
            Self::SwordStrike {
                actor,
                target,
                original_command,
                original_command_name,
                with_seek,
                seek_distance,
            } => TraceCommand::SwordStrike {
                actor,
                target,
                original_command,
                original_command_name,
                with_seek,
                seek_distance: seek_distance.unwrap_or_else(missing_legacy_seek_distance),
            },
            Self::SelectPc { pc, append } => TraceCommand::SelectPc { pc, append },
            Self::UnselectAllPcs => TraceCommand::UnselectAllPcs,
            Self::StopPc { pc } => TraceCommand::StopPc { pc },
            Self::SelectAction {
                pc,
                action,
                original_action,
            } => TraceCommand::SelectAction {
                pc,
                action,
                original_action,
            },
            Self::CancelAction {
                pc,
                action,
                original_action,
            } => TraceCommand::CancelAction {
                pc,
                action,
                original_action,
            },
            Self::OrientActionAt {
                action,
                original_action,
                actor,
                mouse_map,
                target,
            } => TraceCommand::OrientActionAt {
                action,
                original_action,
                actor,
                mouse_map,
                target,
            },
            Self::MakePcFast { entity } => TraceCommand::MakePcFast { entity },
            Self::CrouchDown => TraceCommand::CrouchDown,
            Self::StandUp => TraceCommand::StandUp,
            Self::DropAleAt {
                actor,
                target,
                running,
            } => TraceCommand::DropAleAt {
                actor,
                target,
                running,
            },
            Self::ShieldSelectProtected {
                actor,
                protected_pc,
            } => TraceCommand::ShieldSelectProtected {
                actor,
                protected_pc,
            },
            Self::BoxUnselect {
                first,
                second,
                append,
            } => TraceCommand::BoxUnselect {
                first,
                second,
                append,
            },
            Self::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => TraceCommand::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            },
            Self::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            } => TraceCommand::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            },
            Self::SelectAllPcs => TraceCommand::SelectAllPcs,
            Self::UnselectPc { pc } => TraceCommand::UnselectPc { pc },
            Self::SelectActionIndex { index } => TraceCommand::SelectActionIndex { index },
            Self::SetLockAlt { on } => TraceCommand::SetLockAlt { on },
            Self::KeyControl => TraceCommand::KeyControl,
            Self::KeyReleaseControl => TraceCommand::KeyReleaseControl,
            Self::StartMacro { pc, slot } => TraceCommand::StartMacro { pc, slot },
            Self::DeleteMacro { pc, slot } => TraceCommand::DeleteMacro { pc, slot },
            Self::StartRecordingMacro { pc, slot } => {
                TraceCommand::StartRecordingMacro { pc, slot }
            }
            Self::ChangeQaMemory { slot } => TraceCommand::ChangeQaMemory { slot },
            Self::HeroRefusedAction {
                actor,
                action,
                original_action,
                target,
                reason,
            } => TraceCommand::HeroRefusedAction {
                actor,
                action,
                original_action,
                target,
                reason,
            },
            Self::BeggarDontTalkStamp { entity } => TraceCommand::BeggarDontTalkStamp { entity },
        }
    }
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceElementV66 {
    pub(super) entity_id: TraceEntityId,
    pub(super) creation_order: u32,
    pub(super) class_id: u16,
    pub(super) kind: TraceEntityKind,
    pub(super) active: bool,
    pub(super) blipped: bool,
    pub(super) unreachable: bool,
    pub(super) surface_id: u32,
    pub(super) posture: u32,
    pub(super) position_map: TracePoint,
    pub(super) old_position_map: TracePoint,
    pub(super) position_goal_map: TracePoint,
    pub(super) elevation: TraceFloat,
    pub(super) old_elevation: TraceFloat,
    pub(super) increment_map: TracePoint,
    pub(super) increment_map_valid: Option<bool>,
    pub(super) movement_map: TracePoint,
    pub(super) layer: u16,
    pub(super) layer_goal: u16,
    pub(super) sector: u16,
    pub(super) direction: i16,
    pub(super) direction_goal: i16,
    pub(super) moving: bool,
    pub(super) moving_map: bool,
    pub(super) sprite_row: u16,
    pub(super) sprite_frame: u16,
    pub(super) sprite_frame_count: Option<u16>,
    pub(super) actor: Option<TraceActorV66>,
    pub(super) human: Option<TraceHumanV66>,
    pub(super) pc: Option<TraceElementPc>,
    pub(super) ai: Option<TraceAiV66>,
    pub(super) detection: Option<TraceDetection>,
    pub(super) runtime: Option<TraceJsonValue>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceActorV66 {
    pub(super) action_state: u32,
    pub(super) animation: u32,
    pub(super) command: u16,
    pub(super) command_name: String,
    pub(super) motion_state: u32,
    pub(super) wait_time: u32,
    pub(super) passing_door_directly: Option<bool>,
    pub(super) active_pass_door: Option<Option<TracePassDoor>>,
    pub(super) sequence_element: Option<Option<TraceSequenceElement>>,
    pub(super) position_interface: Option<TraceJsonValue>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceHumanV66 {
    pub(super) life_points: i16,
    pub(super) dead: bool,
    pub(super) unconscious: bool,
    pub(super) camp: String,
    pub(super) original_camp: i32,
    pub(super) vip: bool,
    pub(super) civilian: bool,
    pub(super) opponents: Option<Vec<TraceEntityId>>,
    pub(super) opponent_jump_lines: Option<Vec<Option<TraceJumpLine>>>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceAiV66 {
    pub(super) state: u32,
    pub(super) substate: u32,
    pub(super) script_locked: Option<bool>,
    pub(super) locked: Option<bool>,
    pub(super) locks: Option<u8>,
    pub(super) was_busy: Option<bool>,
    pub(super) very_busy: Option<bool>,
    pub(super) macro_timer_running: Option<bool>,
    pub(super) macro_timer_ring: Option<u32>,
    pub(super) macro_cursor: Option<Option<u16>>,
    pub(super) macro_remaining: Option<u16>,
    pub(super) macro_in_progress: Option<bool>,
    pub(super) list_us: Option<Vec<TraceEntityId>>,
    pub(super) list_them: Option<Vec<TraceEntityId>>,
    pub(super) my_line_jump: Option<Option<TraceJumpLine>>,
}

impl TraceActorV66 {
    pub(super) fn into_current(self) -> TraceActor {
        TraceActor {
            action_state: self.action_state,
            animation: self.animation,
            command: self.command,
            command_name: self.command_name,
            motion_state: self.motion_state,
            wait_time: self.wait_time,
            passing_door_directly: self.passing_door_directly.unwrap_or(false),
            active_pass_door: self.active_pass_door.flatten(),
            sequence_element: self.sequence_element.flatten(),
            position_interface: self
                .position_interface
                .unwrap_or_else(missing_legacy_trace_json_value),
        }
    }
}

impl TraceHumanV66 {
    pub(super) fn into_current(self) -> TraceHuman {
        TraceHuman {
            life_points: self.life_points,
            dead: self.dead,
            unconscious: self.unconscious,
            camp: self.camp,
            original_camp: self.original_camp,
            vip: self.vip,
            civilian: self.civilian,
            opponents: self.opponents.unwrap_or_default(),
            opponent_jump_lines: self.opponent_jump_lines.unwrap_or_default(),
        }
    }
}

impl TraceAiV66 {
    pub(super) fn into_current(self) -> TraceAi {
        TraceAi {
            state: self.state,
            substate: self.substate,
            script_locked: self.script_locked.unwrap_or(false),
            locked: self.locked.unwrap_or(false),
            locks: self.locks.unwrap_or(0),
            was_busy: self.was_busy.unwrap_or(false),
            very_busy: self.very_busy.unwrap_or(false),
            macro_timer_running: self.macro_timer_running.unwrap_or(false),
            macro_timer_ring: self.macro_timer_ring.unwrap_or(0),
            macro_cursor: self.macro_cursor.flatten(),
            macro_remaining: self.macro_remaining.unwrap_or(0),
            macro_in_progress: self.macro_in_progress.unwrap_or(false),
            list_us: self.list_us.unwrap_or_default(),
            list_them: self.list_them.unwrap_or_default(),
            my_line_jump: self.my_line_jump.flatten(),
        }
    }
}

impl TraceElementV66 {
    pub(super) fn into_current(self) -> TraceElement {
        TraceElement {
            entity_id: self.entity_id,
            creation_order: self.creation_order,
            class_id: self.class_id,
            kind: self.kind,
            active: self.active,
            blipped: self.blipped,
            unreachable: self.unreachable,
            surface_id: self.surface_id,
            posture: self.posture,
            position_map: self.position_map,
            old_position_map: self.old_position_map,
            position_goal_map: self.position_goal_map,
            elevation: self.elevation,
            old_elevation: self.old_elevation,
            increment_map: self.increment_map,
            increment_map_valid: self.increment_map_valid,
            movement_map: self.movement_map,
            layer: self.layer,
            layer_goal: self.layer_goal,
            sector: self.sector,
            direction: self.direction,
            direction_goal: self.direction_goal,
            moving: self.moving,
            moving_map: self.moving_map,
            sprite_row: self.sprite_row,
            sprite_frame: self.sprite_frame,
            sprite_frame_count: self.sprite_frame_count.unwrap_or(0),
            actor: self.actor.map(TraceActorV66::into_current),
            human: self.human.map(TraceHumanV66::into_current),
            pc: self.pc,
            ai: self.ai.map(TraceAiV66::into_current),
            detection: self.detection,
            runtime: self.runtime.unwrap_or_else(missing_legacy_trace_json_value),
        }
    }
}

/// Frame layout embedded in native trace version 66.
#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceFrameV66 {
    #[serde(rename = "type")]
    pub(super) record_type: String,
    pub(super) frame_before: u64,
    pub(super) frame_after: u64,
    pub(super) game_code: i32,
    pub(super) simulation_body_ran: bool,
    pub(super) commands: Vec<TraceCommandV66>,
    pub(super) director_completions: Vec<TraceDirectorCompletion>,
    pub(super) campaign: Option<TraceCampaign>,
    pub(super) engine_state: Option<TraceEngineStateV66>,
    pub(super) selected_pcs: Vec<TraceEntityId>,
    pub(super) elements: Vec<TraceElementV66>,
    pub(super) visibility_queries: Vec<TraceVisibilityQuery>,
    pub(super) rng_draws: TraceRngBatch,
    pub(super) motion_line_changes: Vec<TraceMotionLineChange>,
    pub(super) path_events: Vec<TracePathEvent>,
    pub(super) route_construction_events: Option<Vec<TraceRouteConstructionEvent>>,
    pub(super) popup_events: Option<Vec<TracePopupEvent>>,
    pub(super) ai_forecast_events: Option<Vec<TraceAiForecastEvent>>,
    pub(super) alert_formation_events: Option<Vec<TraceAlertFormationEvent>>,
    pub(super) goto_authorization_events: Option<Vec<TraceGoToAuthorizationEvent>>,
    pub(super) strike_proposal_events: Option<Vec<TraceStrikeProposalEvent>>,
    pub(super) sequence_lifecycle_events: Option<Vec<TraceSequenceLifecycleEvent>>,
    pub(super) target_lifecycle_events: Option<Vec<TraceTargetLifecycleEvent>>,
    pub(super) resolved_exclamations: Vec<TraceResolvedExclamation>,
    pub(super) movement_steps: Vec<TraceMovementStep>,
    pub(super) flight_steps: Vec<TraceFlightStep>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceEngineStateV66 {
    pub(super) cheat_used_flags: u32,
    pub(super) next_creation_order: u32,
    pub(super) chorus_timer: u16,
    pub(super) force_check: bool,
    pub(super) men_to_blazon_conversion: bool,
    pub(super) game_ui: Option<TraceJsonValue>,
    pub(super) messenger_controller: Option<TraceJsonValue>,
    pub(super) shield_controller: Option<TraceJsonValue>,
    pub(super) pc_registry: TraceJsonValue,
    pub(super) lock_engine: bool,
    pub(super) freeze_all: bool,
    pub(super) locker: bool,
    pub(super) speed: TraceFloat,
    pub(super) speed_int: u16,
    pub(super) mission_won: bool,
    pub(super) mission_won_first_time: bool,
    pub(super) quit_won: bool,
    pub(super) quit_lost: bool,
    pub(super) quit_interrupted: bool,
    pub(super) script_globals: Vec<i32>,
    pub(super) sequence_manager: TraceJsonValue,
    pub(super) script_runtime: TraceJsonValue,
    pub(super) pathfinder: TraceJsonValue,
    pub(super) view_radius_cache: TraceJsonValue,
    pub(super) sound_sources: TraceJsonValue,
    pub(super) sound_completion_frontier: Option<TraceJsonValue>,
    pub(super) ai_global: TraceJsonValue,
    pub(super) engine_runtime_roots: TraceJsonValue,
    pub(super) world_interactables: TraceJsonValue,
    pub(super) repulsive_points: TraceJsonValue,
    pub(super) titbit_manager: TraceJsonValue,
    pub(super) failed_path_requests: Vec<TraceFailedPathRequestV66>,
}

#[derive(Debug, Serialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceFailedPathRequestV66 {
    pub(super) actor: TraceEntityId,
    pub(super) antagonist: Option<TraceEntityId>,
    pub(super) layer: u16,
    pub(super) area: u16,
    pub(super) source: TracePoint,
    pub(super) goal: TracePoint,
    pub(super) half_diagonal_index: u16,
    pub(super) half_diagonal: TracePoint,
    pub(super) animation: u32,
    pub(super) reverse: bool,
    pub(super) speed: u8,
    pub(super) tolerance: TraceFloat,
    pub(super) use_first_point: bool,
    pub(super) sector: u16,
    pub(super) time: u32,
}

impl TraceFrameV66 {
    pub(super) fn into_current(self) -> TraceFrame {
        TraceFrame {
            record_type: self.record_type,
            frame_before: self.frame_before,
            frame_after: self.frame_after,
            game_code: self.game_code,
            simulation_body_ran: self.simulation_body_ran,
            commands: self
                .commands
                .into_iter()
                .map(TraceCommandV66::into_current)
                .collect(),
            director_completions: self.director_completions,
            selected_pcs: self.selected_pcs,
            elements: self
                .elements
                .into_iter()
                .map(TraceElementV66::into_current)
                .collect(),
            visibility_queries: self.visibility_queries,
            rng_draws: self.rng_draws,
            motion_line_changes: self.motion_line_changes,
            path_events: self.path_events,
            route_construction_events: self.route_construction_events.unwrap_or_default(),
            popup_events: self.popup_events.unwrap_or_default(),
            ai_forecast_events: self.ai_forecast_events.unwrap_or_default(),
            alert_formation_events: self.alert_formation_events.unwrap_or_default(),
            goto_authorization_events: self.goto_authorization_events.unwrap_or_default(),
            strike_proposal_events: self.strike_proposal_events.unwrap_or_default(),
            sequence_lifecycle_events: self.sequence_lifecycle_events.unwrap_or_default(),
            target_lifecycle_events: self.target_lifecycle_events.unwrap_or_default(),
            resolved_exclamations: self.resolved_exclamations,
            movement_steps: self.movement_steps,
            flight_steps: self.flight_steps,
        }
    }
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct BinaryTraceHeaderV66 {
    pub(super) version: u32,
    pub(super) source_fingerprint: String,
    pub(super) trace: TraceHeaderV66,
    pub(super) rng_prefix: TraceRngPrefix,
}

impl From<BinaryTraceHeaderV66> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV66) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV66 {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_V66_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) enum BinaryTraceRecordV66 {
    Frame(TraceFrameV66),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

impl BinaryTraceRecordV66 {
    pub(super) fn into_current(self) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => BinaryTraceRecord::Frame(frame.into_current()),
            Self::End {
                rng_suffix,
                final_frame,
                frame_count,
            } => BinaryTraceRecord::End {
                rng_suffix,
                final_frame,
                frame_count,
            },
        }
    }
}
