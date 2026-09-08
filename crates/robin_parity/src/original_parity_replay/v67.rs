//! Frozen v67 wire layouts and one-way compatibility conversion.
//!
//! Field and variant order, field types, and shared child layouts are part of
//! authoritative bitcode artifacts. Do not modernize these shapes. Conversion
//! may reconstruct only the omissions documented by this generation. Reverse
//! conversions are test-only: production always writes the current layout.
use super::*;

/// Header layout written by native trace version 67. Changing
/// `initial_npc_transients` from `Vec<_>` to `Option<Vec<_>>` changes
/// bitcode's struct layout and therefore requires native trace version 68.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceHeaderV67 {
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
    pub(super) random_input_seed: Option<u32>,
    pub(super) sim_config: TraceSimConfig,
    pub(super) campaign: TraceCampaign,
    pub(super) motion_grid: TraceMotionGrid,
    pub(super) initial_npc_transients: Vec<TraceInitialNpcTransient>,
    pub(super) initial_save: Option<TraceInitialSave>,
}

impl From<TraceHeaderV67> for TraceHeader {
    fn from(header: TraceHeaderV67) -> Self {
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
            // V67 could not distinguish an omitted JSON field from a present
            // empty array. Its replay semantics treated both as legacy state.
            initial_npc_transients: (!header.initial_npc_transients.is_empty())
                .then_some(header.initial_npc_transients),
            initial_save: header.initial_save,
        }
    }
}

#[cfg(test)]
impl From<TraceHeader> for TraceHeaderV67 {
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
            random_input_seed: header.random_input_seed,
            sim_config: header.sim_config,
            campaign: header.campaign,
            motion_grid: header.motion_grid,
            initial_npc_transients: header.initial_npc_transients.unwrap_or_default(),
            initial_save: header.initial_save,
        }
    }
}

/// Element snapshot layout embedded in native trace version 67. Keep this
/// frozen: even a field-order-only edit changes bitcode's on-disk shape.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceElementV67 {
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
    pub(super) increment_map_valid: bool,
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
    pub(super) sprite_frame_count: u16,
    pub(super) actor: Option<TraceActor>,
    pub(super) human: Option<TraceHuman>,
    pub(super) pc: Option<TraceElementPc>,
    pub(super) ai: Option<TraceAi>,
    pub(super) detection: Option<TraceDetection>,
    pub(super) runtime: TraceJsonValue,
}

impl TraceElementV67 {
    pub(super) fn into_current(self, increment_map_valid_was_recorded: bool) -> TraceElement {
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
            increment_map_valid: increment_map_valid_was_recorded
                .then_some(self.increment_map_valid),
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
            sprite_frame_count: self.sprite_frame_count,
            actor: self.actor,
            human: self.human,
            pc: self.pc,
            ai: self.ai,
            detection: self.detection,
            runtime: self.runtime,
        }
    }
}

/// Frame layout embedded in native trace version 67. Its element type retains
/// the original plain-`bool` `increment_map_valid` field.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct TraceFrameV67 {
    pub(super) record_type: String,
    pub(super) frame_before: u64,
    pub(super) frame_after: u64,
    pub(super) game_code: i32,
    pub(super) simulation_body_ran: bool,
    pub(super) commands: Vec<TraceCommand>,
    pub(super) director_completions: Vec<TraceDirectorCompletion>,
    pub(super) selected_pcs: Vec<TraceEntityId>,
    pub(super) elements: Vec<TraceElementV67>,
    pub(super) visibility_queries: Vec<TraceVisibilityQuery>,
    pub(super) rng_draws: TraceRngBatch,
    pub(super) motion_line_changes: Vec<TraceMotionLineChange>,
    pub(super) path_events: Vec<TracePathEvent>,
    pub(super) route_construction_events: Vec<TraceRouteConstructionEvent>,
    pub(super) popup_events: Vec<TracePopupEvent>,
    pub(super) ai_forecast_events: Vec<TraceAiForecastEvent>,
    pub(super) alert_formation_events: Vec<TraceAlertFormationEvent>,
    pub(super) goto_authorization_events: Vec<TraceGoToAuthorizationEvent>,
    pub(super) strike_proposal_events: Vec<TraceStrikeProposalEvent>,
    pub(super) sequence_lifecycle_events: Vec<TraceSequenceLifecycleEvent>,
    pub(super) target_lifecycle_events: Vec<TraceTargetLifecycleEvent>,
    pub(super) resolved_exclamations: Vec<TraceResolvedExclamation>,
    pub(super) movement_steps: Vec<TraceMovementStep>,
    pub(super) flight_steps: Vec<TraceFlightStep>,
}

impl TraceFrameV67 {
    pub(super) fn into_current(self, increment_map_valid_was_recorded: bool) -> TraceFrame {
        TraceFrame {
            record_type: self.record_type,
            frame_before: self.frame_before,
            frame_after: self.frame_after,
            game_code: self.game_code,
            simulation_body_ran: self.simulation_body_ran,
            commands: self.commands,
            director_completions: self.director_completions,
            selected_pcs: self.selected_pcs,
            elements: self
                .elements
                .into_iter()
                .map(|element| element.into_current(increment_map_valid_was_recorded))
                .collect(),
            visibility_queries: self.visibility_queries,
            rng_draws: self.rng_draws,
            motion_line_changes: self.motion_line_changes,
            path_events: self.path_events,
            route_construction_events: self.route_construction_events,
            popup_events: self.popup_events,
            ai_forecast_events: self.ai_forecast_events,
            alert_formation_events: self.alert_formation_events,
            goto_authorization_events: self.goto_authorization_events,
            strike_proposal_events: self.strike_proposal_events,
            sequence_lifecycle_events: self.sequence_lifecycle_events,
            target_lifecycle_events: self.target_lifecycle_events,
            resolved_exclamations: self.resolved_exclamations,
            movement_steps: self.movement_steps,
            flight_steps: self.flight_steps,
        }
    }
}

#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) struct BinaryTraceHeaderV67 {
    pub(super) version: u32,
    pub(super) source_fingerprint: String,
    pub(super) trace: TraceHeaderV67,
    pub(super) rng_prefix: TraceRngPrefix,
}

impl From<BinaryTraceHeaderV67> for BinaryTraceHeaderV68 {
    fn from(header: BinaryTraceHeaderV67) -> Self {
        Self {
            version: header.version,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

#[cfg(test)]
impl From<BinaryTraceHeaderV68> for BinaryTraceHeaderV67 {
    fn from(header: BinaryTraceHeaderV68) -> Self {
        Self {
            version: TRACE_NATIVE_LEGACY_VERSION,
            source_fingerprint: header.source_fingerprint,
            trace: header.trace.into(),
            rng_prefix: header.rng_prefix,
        }
    }
}

/// Record layout written by native trace version 67. Do not modify this type
/// or any of its versioned children.
#[derive(Debug, bitcode::Encode, bitcode::Decode)]
pub(super) enum BinaryTraceRecordV67 {
    Frame(TraceFrameV67),
    End {
        rng_suffix: Option<TraceRngBatch>,
        final_frame: Option<u64>,
        frame_count: Option<u64>,
    },
}

impl BinaryTraceRecordV67 {
    pub(super) fn into_current(self, increment_map_valid_was_recorded: bool) -> BinaryTraceRecord {
        match self {
            Self::Frame(frame) => {
                BinaryTraceRecord::Frame(frame.into_current(increment_map_valid_was_recorded))
            }
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
