//! Additive per-frame diagnostic event streams (path, route, popup, AI, lifecycle, motion).
use super::json::TraceJsonValue;
use super::scalar::{TraceEntityId, TraceFloat, TracePoint, TracePoint3};
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum TracePathEvent {
    Queued {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
    },
    Completed {
        actor: TraceEntityId,
        antagonist: Option<TraceEntityId>,
        layer: u16,
        area: u16,
        source: TracePoint,
        goal: TracePoint,
        half_diagonal_index: u16,
        half_diagonal: TracePoint,
        animation: u32,
        reverse: bool,
        speed: u8,
        tolerance: TraceFloat,
        use_first_point: bool,
        valid: bool,
        waypoints: Vec<TracePoint>,
    },
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceResolvedExclamation {
    pub(crate) actor: TraceEntityId,
    pub(crate) identifier: u32,
    pub(crate) exclamation_id: u16,
    pub(crate) selected_variant: i32,
    pub(crate) selected_entry: Option<u32>,
    pub(crate) duration_frames: u32,
}

/// One exact, ordered original-game motion position commit.
/// This additive diagnostic is absent unless the Original recorder was run
/// with `RH_PARITY_MOVEMENT_STEPS` enabled.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceMovementStep {
    pub(crate) entity: TraceEntityId,
    pub(crate) order_id: u32,
    pub(crate) order_action: u32,
    pub(crate) animation: u32,
    pub(crate) motion_method: u32,
    pub(crate) pre_position: TracePoint,
    pub(crate) old_position: TracePoint,
    pub(crate) goal: TracePoint,
    pub(crate) cached_increment: TracePoint,
    pub(crate) frame_distance_raw: TraceFloat,
    pub(crate) speed_factor: TraceFloat,
    pub(crate) effective_distance: TraceFloat,
    pub(crate) anti_collision: bool,
    pub(crate) reverse: bool,
    pub(crate) raw_post_position: TracePoint,
    pub(crate) raw_committed_delta: TracePoint,
    pub(crate) post_position: TracePoint,
    pub(crate) committed_delta: TracePoint,
    pub(crate) goal_reached: bool,
    pub(crate) snapped_to_goal: bool,
}

/// One exact, ordered original-game flight execution.
#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceFlightStep {
    pub(crate) entity: TraceEntityId,
    pub(crate) order_id: u32,
    pub(crate) order_action: u32,
    pub(crate) animation: u32,
    pub(crate) flight_style: u32,
    pub(crate) entry_position: TracePoint3,
    pub(crate) entry_position_map: TracePoint,
    pub(crate) old_position: TracePoint3,
    pub(crate) old_position_map: TracePoint,
    pub(crate) goal: TracePoint3,
    pub(crate) cached_increment: TracePoint3,
    pub(crate) applied_increment: TracePoint3,
    pub(crate) raw_post_position: TracePoint3,
    pub(crate) raw_post_position_map: TracePoint,
    pub(crate) motion_state: u32,
    pub(crate) post_position: TracePoint3,
    pub(crate) post_position_map: TracePoint,
    pub(crate) snapped_to_goal: bool,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceRouteConstructionEvent {
    pub(crate) kind: String,
    pub(crate) actor: TraceEntityId,
    pub(crate) source: TracePoint,
    pub(crate) source_sector: u16,
    pub(crate) source_level: u16,
    pub(crate) goal: TracePoint,
    pub(crate) goal_sector: u16,
    pub(crate) goal_level: u16,
    pub(crate) gates: Vec<TraceRouteGate>,
    /// Schema-16 may extend route events while its diagnostic contract is
    /// being exercised against real recordings. Retain every additive field
    /// in the native cache instead of silently discarding useful evidence.
    #[serde(flatten)]
    pub(crate) draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceRouteGate {
    pub(crate) gate_id: u32,
    pub(crate) direct: bool,
    pub(crate) sector_out: u16,
    pub(crate) level_out: u16,
    pub(crate) sector_in: u16,
    pub(crate) level_in: u16,
    #[serde(flatten)]
    pub(crate) draft_diagnostics: BTreeMap<String, TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TracePointBox {
    pub(crate) top_left: TracePoint,
    pub(crate) bottom_right: TracePoint,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TracePopupEvent {
    #[serde(default)]
    pub(crate) ordinal: Option<u64>,
    pub(crate) stage: String,
    #[serde(default)]
    pub(crate) universal_frame_counter: Option<u64>,
    #[serde(default)]
    pub(crate) last_popup_frame: Option<u64>,
    #[serde(default)]
    pub(crate) last_popup_frame_initialized: Option<bool>,
    #[serde(default)]
    pub(crate) same_frame_suppressed: Option<bool>,
    #[serde(default)]
    pub(crate) colorize_background: Option<bool>,
    #[serde(default)]
    pub(crate) modal: Option<bool>,
    #[serde(default)]
    pub(crate) centered: Option<bool>,
    #[serde(default)]
    pub(crate) popup_text_id: Option<u64>,
    #[serde(default)]
    pub(crate) source_surface: Option<u64>,
    #[serde(default)]
    pub(crate) remove_mouse: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceForecastGate {
    pub(crate) gate_id: u32,
    pub(crate) kind: String,
    pub(crate) active: bool,
    pub(crate) point_out: TracePoint,
    pub(crate) sector_out: u16,
    pub(crate) level_out: u16,
    pub(crate) point_in: TracePoint,
    pub(crate) sector_in: u16,
    pub(crate) level_in: u16,
    pub(crate) penalty: TraceFloat,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceAiForecastInput {
    pub(crate) position: TracePoint,
    pub(crate) sector: u16,
    pub(crate) level: u16,
    #[serde(default)]
    pub(crate) direction: Option<u16>,
    pub(crate) passing_door: bool,
    #[serde(default)]
    pub(crate) passing_door_directly: Option<bool>,
    #[serde(default)]
    pub(crate) door: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceAiForecastResolved {
    pub(crate) position: TracePoint,
    pub(crate) sector: u16,
    pub(crate) level: u16,
    #[serde(default)]
    pub(crate) direction: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceAiForecastEvent {
    pub(crate) ordinal: u64,
    #[serde(default)]
    pub(crate) phase: Option<String>,
    pub(crate) target: TraceEntityId,
    pub(crate) input: TraceAiForecastInput,
    pub(crate) moving_upwards: bool,
    pub(crate) resolution: String,
    pub(crate) resolved: TraceAiForecastResolved,
    #[serde(default)]
    pub(crate) selected_building_exit: Option<TraceForecastGate>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceAlertEligibility {
    pub(crate) rank: bool,
    pub(crate) able_to_help: Option<bool>,
    pub(crate) allowed_to_leave_post: Option<bool>,
    pub(crate) can_call: Option<bool>,
    pub(crate) max_radius: Option<bool>,
    pub(crate) squared_radius: Option<bool>,
    pub(crate) capacity: Option<bool>,
    pub(crate) think: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceAlertFormationEvent {
    #[serde(default)]
    pub(crate) ordinal: Option<u64>,
    pub(crate) stage: String,
    pub(crate) invocation: u64,
    #[serde(default)]
    pub(crate) officer: Option<TraceEntityId>,
    #[serde(default)]
    pub(crate) officer_position: Option<TracePoint>,
    #[serde(default)]
    pub(crate) soldier_scan_count: Option<u16>,
    #[serde(default)]
    pub(crate) officer_in_building: Option<bool>,
    #[serde(default)]
    pub(crate) scan_index: Option<u16>,
    #[serde(default)]
    pub(crate) candidate: Option<TraceEntityId>,
    #[serde(default)]
    pub(crate) active: Option<bool>,
    #[serde(default)]
    pub(crate) script_locked: Option<bool>,
    #[serde(default)]
    pub(crate) eligibility: Option<TraceAlertEligibility>,
    #[serde(default)]
    pub(crate) rejection_stage: Option<String>,
    #[serde(default)]
    pub(crate) insertion_index: Option<u16>,
    #[serde(default)]
    pub(crate) squared_distance: Option<TraceFloat>,
    #[serde(default)]
    pub(crate) normalized_contribution: Option<TracePoint>,
    #[serde(default)]
    pub(crate) running_average: Option<TracePoint>,
    #[serde(default)]
    pub(crate) selected_index: Option<u16>,
    #[serde(default)]
    pub(crate) outside_step: Option<u16>,
    #[serde(default)]
    pub(crate) direction: Option<u16>,
    #[serde(default)]
    pub(crate) soldier_count: Option<u16>,
    #[serde(default)]
    pub(crate) slot_index: Option<u16>,
    #[serde(default)]
    pub(crate) layer: Option<u16>,
    #[serde(default)]
    pub(crate) sector: Option<u16>,
    #[serde(default)]
    pub(crate) destination: Option<TracePoint>,
    #[serde(default)]
    pub(crate) destination_box: Option<TracePointBox>,
    #[serde(default)]
    pub(crate) position_authorized: Option<bool>,
    #[serde(default)]
    pub(crate) thick_corridor_authorized: Option<bool>,
    #[serde(default)]
    pub(crate) blocker_ids_available: Option<bool>,
    #[serde(default)]
    pub(crate) blocking_motion_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    pub(crate) blocking_mobile_line_ids: Option<Vec<u32>>,
    #[serde(default)]
    pub(crate) accepted: Option<bool>,
    #[serde(default)]
    pub(crate) result: Option<String>,
    #[serde(default)]
    pub(crate) average_direction: Option<u16>,
    #[serde(default)]
    pub(crate) selected_direction: Option<u16>,
    #[serde(default)]
    pub(crate) final_sector: Option<u16>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceGoToSource {
    pub(crate) point: TracePoint,
    pub(crate) layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceGoToDestination {
    pub(crate) point: TracePoint,
    pub(crate) sector: Option<u16>,
    pub(crate) layer: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceGoToAuthorizationEvent {
    pub(crate) ordinal: u64,
    pub(crate) actor: Option<TraceEntityId>,
    #[serde(default)]
    pub(crate) source: Option<TraceGoToSource>,
    #[serde(default)]
    pub(crate) move_box: Option<TracePointBox>,
    pub(crate) destination: TraceGoToDestination,
    pub(crate) requested_flags: u16,
    pub(crate) effective_flags: u16,
    pub(crate) phase: String,
    pub(crate) outcome: String,
    pub(crate) straight_authorized: Option<bool>,
    pub(crate) path_authorized: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TraceTargetLifecyclePayload {
    ActivateSword,
    PlayAnimFreeze {
        animation_id: Option<u32>,
    },
    SendMessage {
        message: Option<u32>,
        argument: Option<i32>,
        argument_raw: Option<u32>,
        extended_argument: Option<i32>,
        extended_argument_raw: Option<u32>,
    },
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceTargetLifecycleEvent {
    pub(crate) ordinal: u64,
    #[serde(default)]
    pub(crate) frame_ordinal: Option<u64>,
    pub(crate) phase: String,
    pub(crate) sequence_id: Option<u32>,
    pub(crate) sequence_element_id: u32,
    pub(crate) command_level: u16,
    pub(crate) state: u32,
    pub(crate) command: u16,
    pub(crate) command_name: String,
    pub(crate) owner: Option<TraceEntityId>,
    pub(crate) context: Option<TraceEntityId>,
    pub(crate) antagonist: Option<TraceEntityId>,
    #[serde(default)]
    pub(crate) antagonist_observed: Option<bool>,
    pub(crate) payload: TraceTargetLifecyclePayload,
    #[serde(default)]
    pub(crate) payload_observed: Option<bool>,
    pub(crate) script_enabled: Option<bool>,
    pub(crate) class_instantiated: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceStrikeProposalEvent {
    pub(crate) invocation: u32,
    pub(crate) ordinal: u64,
    pub(crate) frame_ordinal: u64,
    pub(crate) phase: String,
    pub(crate) actor: Option<TraceEntityId>,
    pub(crate) actor_creation_order: u32,
    pub(crate) threat: Option<TraceEntityId>,
    pub(crate) threat_creation_order: Option<u32>,
    pub(crate) principal_opponent: Option<TraceEntityId>,
    pub(crate) principal_opponent_creation_order: Option<u32>,
    pub(crate) command: Option<u16>,
    pub(crate) command_name: Option<String>,
    pub(crate) also_parade: Option<bool>,
    pub(crate) only_parade: Option<bool>,
    pub(crate) fighting_ability: Option<u16>,
    pub(crate) blood_alcohol: Option<u16>,
    pub(crate) special_gate_fighting_ability: Option<u16>,
    pub(crate) parade_gate_fighting_ability: Option<u16>,
    pub(crate) first_random_raw: Option<u32>,
    pub(crate) first_random_modulo: Option<u32>,
    pub(crate) second_random_raw: Option<u32>,
    pub(crate) second_random_modulo: Option<u32>,
    pub(crate) reason: Option<String>,
    pub(crate) opponent_animation: Option<u32>,
    pub(crate) opponent_strike: Option<i32>,
    pub(crate) candidate_strike: Option<i32>,
    pub(crate) time_limit: Option<i32>,
    pub(crate) minimum_skill: Option<u16>,
    pub(crate) maximum_alcohol: Option<u16>,
    pub(crate) boredom_before: Option<u16>,
    pub(crate) boredom_after_decay: Option<u16>,
    pub(crate) skill_eligible: Option<bool>,
    pub(crate) alcohol_eligible: Option<bool>,
    pub(crate) time_eligible: Option<bool>,
    pub(crate) raw_damage: Option<i32>,
    pub(crate) victim_count: Option<i32>,
    pub(crate) boredom_penalty: Option<i32>,
    pub(crate) drunken_bonus: Option<i32>,
    pub(crate) adjusted_damage: Option<i32>,
    pub(crate) group_strike: Option<bool>,
    pub(crate) group_condition: Option<bool>,
    pub(crate) accepted_as_best: Option<bool>,
    pub(crate) selected_strike: Option<i32>,
    pub(crate) parry_transition_frames: Option<i32>,
    pub(crate) parry_time_eligible: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceSequenceLifecycleEvent {
    pub(crate) ordinal: u64,
    pub(crate) frame_ordinal: u64,
    pub(crate) event: String,
    pub(crate) phase: String,
    pub(crate) element_id: u32,
    pub(crate) sequence_id: Option<u32>,
    pub(crate) owner: Option<TraceEntityId>,
    pub(crate) owner_creation_order: Option<u32>,
    pub(crate) command: u16,
    pub(crate) command_name: Option<String>,
    pub(crate) command_level: u16,
    pub(crate) state: Option<u32>,
    pub(crate) priority: Option<u32>,
    pub(crate) queue_size_before: Option<u32>,
    pub(crate) queue_size_after: Option<u32>,
    pub(crate) actor: Option<TraceEntityId>,
    pub(crate) actor_creation_order: Option<u32>,
    pub(crate) selected_sequence_id: Option<u32>,
    pub(crate) selected_command: Option<u16>,
    pub(crate) current_order_id: Option<u32>,
    pub(crate) current_order_action: Option<u32>,
    pub(crate) decision: Option<i32>,
    pub(crate) accepted: Option<bool>,
}
