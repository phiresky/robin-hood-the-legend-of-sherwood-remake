//! Per-frame element state: actors, humans, AI, detection and visibility queries.
use super::json::{TraceJsonValue, missing_legacy_trace_json_value};
use super::scalar::{TraceEntityId, TraceEntityKind, TraceFloat, TracePoint, TracePoint3};
use bitcode_parity as bitcode;
use serde::{Deserialize, Serialize};

/// Element layout embedded in version-68 native frame records.
///
/// ON-DISK FORMAT INVARIANT: do not change fields, their order, or their
/// types without bumping `TRACE_NATIVE_VERSION` and freezing this layout in a
/// version-named compatibility type (as the since-deleted `TraceElementV67`
/// used to be for version 67).
#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceElement {
    pub(crate) entity_id: TraceEntityId,
    pub(crate) creation_order: u32,
    pub(crate) class_id: u16,
    pub(crate) kind: TraceEntityKind,
    pub(crate) active: bool,
    pub(crate) blipped: bool,
    pub(crate) unreachable: bool,
    pub(crate) surface_id: u32,
    pub(crate) posture: u32,
    pub(crate) position_map: TracePoint,
    pub(crate) old_position_map: TracePoint,
    pub(crate) position_goal_map: TracePoint,
    pub(crate) elevation: TraceFloat,
    pub(crate) old_elevation: TraceFloat,
    pub(crate) increment_map: TracePoint,
    /// Missing in early schema-16 frames. Presence, including an authoritative
    /// `false`, must survive conversion to the native trace.
    #[serde(default)]
    pub(crate) increment_map_valid: Option<bool>,
    pub(crate) movement_map: TracePoint,
    pub(crate) layer: u16,
    pub(crate) layer_goal: u16,
    pub(crate) sector: u16,
    pub(crate) direction: i16,
    pub(crate) direction_goal: i16,
    pub(crate) moving: bool,
    pub(crate) moving_map: bool,
    pub(crate) sprite_row: u16,
    pub(crate) sprite_frame: u16,
    pub(crate) sprite_frame_count: u16,
    #[serde(default)]
    pub(crate) actor: Option<TraceActor>,
    #[serde(default)]
    pub(crate) human: Option<TraceHuman>,
    #[serde(default)]
    pub(crate) pc: Option<TraceElementPc>,
    #[serde(default)]
    pub(crate) ai: Option<TraceAi>,
    #[serde(default)]
    pub(crate) detection: Option<TraceDetection>,
    /// Whole-entity serialized position/sprite frontier. Early schema-16
    /// recordings omit it; JSON null is the native-layout-compatible marker
    /// for "not recorded" and is excluded from logical comparison.
    #[serde(default = "missing_legacy_trace_json_value")]
    pub(crate) runtime: TraceJsonValue,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceActor {
    pub(crate) action_state: u32,
    pub(crate) animation: u32,
    pub(crate) command: u16,
    pub(crate) command_name: String,
    pub(crate) motion_state: u32,
    pub(crate) wait_time: u32,
    #[serde(default)]
    pub(crate) passing_door_directly: bool,
    /// Explicitly null when there is no active PassDoor.
    #[serde(default)]
    pub(crate) active_pass_door: Option<TracePassDoor>,
    /// Rust does not yet expose a stable public current-sequence snapshot with
    /// Original's element identities.
    /// TODO(parity-sequence): compare the remaining fields once that capture
    /// can be produced without walking mutable sequence-manager internals.
    #[serde(default)]
    pub(crate) sequence_element: Option<TraceSequenceElement>,
    /// PositionInterface diagnostics. Kept as a cache-safe JSON
    /// tree because it is observational evidence rather than comparable
    /// engine state yet.
    #[serde(default = "missing_legacy_trace_json_value")]
    pub(crate) position_interface: TraceJsonValue,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TracePassDoor {
    pub(crate) gate_id: u32,
    pub(crate) direct: bool,
    pub(crate) direction: i16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSequenceElement {
    pub(crate) id: u32,
    #[serde(rename = "type")]
    pub(crate) element_type: u8,
    pub(crate) state: u32,
    pub(crate) command_level: u16,
    pub(crate) command: u16,
    pub(crate) command_name: String,
    pub(crate) order_count: u16,
    pub(crate) priority: u32,
    pub(crate) posture_after_transition: u32,
    pub(crate) action_state_after_transition: u32,
    #[serde(default)]
    pub(crate) movement: Option<TraceSequenceMovement>,
    /// Current sequence topology and active-order diagnostics. These are
    /// nullable or command-shaped in the Original recorder, so retaining the
    /// draft payload verbatim is safer than inventing a false common shape.
    #[serde(default)]
    pub(crate) following: Option<TraceJsonValue>,
    #[serde(default)]
    pub(crate) postponed: Option<TraceJsonValue>,
    #[serde(default)]
    pub(crate) current_order: Option<TraceJsonValue>,
    #[serde(default)]
    pub(crate) movement_payload: Option<TraceJsonValue>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSequenceMovement {
    /// Absent in current schema-16 traces when the movement-element
    /// constructor does not initialize `maction` (for example WAIT_FREE_LIFT).
    #[serde(default)]
    pub(crate) action: Option<u32>,
    #[serde(default)]
    pub(crate) pass_door: Option<TracePassDoor>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceHuman {
    pub(crate) life_points: i16,
    pub(crate) dead: bool,
    pub(crate) unconscious: bool,
    pub(crate) camp: String,
    pub(crate) original_camp: i32,
    pub(crate) vip: bool,
    pub(crate) civilian: bool,
    pub(crate) opponents: Vec<TraceEntityId>,
    pub(crate) opponent_jump_lines: Vec<Option<TraceJumpLine>>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceJumpLine {
    pub(crate) a: TracePoint,
    pub(crate) b: TracePoint,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceElementPc {
    pub(crate) ammo: TraceElementAmmo,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceElementAmmo {
    pub(crate) ales: u16,
    pub(crate) apples: u16,
    pub(crate) arrows: u16,
    pub(crate) nets: u16,
    pub(crate) plants: u16,
    pub(crate) purses: u16,
    pub(crate) rations: u16,
    pub(crate) stones: u16,
    pub(crate) wasp_nests: u16,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceAi {
    pub(crate) state: u32,
    pub(crate) substate: u32,
    #[serde(default)]
    pub(crate) script_locked: bool,
    #[serde(default)]
    pub(crate) locked: bool,
    #[serde(default)]
    pub(crate) locks: u8,
    #[serde(default)]
    pub(crate) was_busy: bool,
    #[serde(default)]
    pub(crate) very_busy: bool,
    #[serde(default)]
    pub(crate) macro_timer_running: bool,
    #[serde(default)]
    pub(crate) macro_timer_ring: u32,
    /// Explicitly null for an inactive macro.
    #[serde(default)]
    pub(crate) macro_cursor: Option<u16>,
    #[serde(default)]
    pub(crate) macro_remaining: u16,
    #[serde(default)]
    pub(crate) macro_in_progress: bool,
    #[serde(default)]
    pub(crate) list_us: Vec<TraceEntityId>,
    #[serde(default)]
    pub(crate) list_them: Vec<TraceEntityId>,
    /// Authoritative jump-line reference, explicitly null when absent.
    #[serde(default)]
    pub(crate) my_line_jump: Option<TraceJumpLine>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceDetection {
    pub(crate) suspects: Vec<u16>,
    pub(crate) maximal_suspect: u16,
    pub(crate) maximal_visibility: u32,
    pub(crate) view_status: u8,
    pub(crate) alert_status: u32,
    pub(crate) detectables: Vec<TraceDetectable>,
}

#[derive(Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceDetectable {
    #[serde(rename = "type")]
    pub(crate) detectable_type: u32,
    pub(crate) target: TraceEntityId,
    pub(crate) seen_now: bool,
    pub(crate) seen_last_frame: bool,
    pub(crate) heard_last_frame: bool,
    pub(crate) shadow_seen_now: bool,
    pub(crate) shadow_seen_last_frame: bool,
    pub(crate) last_visibility: TraceFloat,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceVisibilityQuery {
    pub(crate) origin: TracePoint3,
    pub(crate) destination: TracePoint3,
    pub(crate) result: bool,
    pub(crate) cache_hit: bool,
    pub(crate) cache_key: u64,
    pub(crate) cache_offset: u64,
    pub(crate) candidate_count: u16,
    pub(crate) reason: String,
    pub(crate) blocking_obstacle: Option<TraceSightObstacle>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSightObstacle {
    pub(crate) id: u32,
    pub(crate) index: i64,
    pub(crate) type_mask: i32,
    pub(crate) types: TraceSightObstacleTypes,
    pub(crate) active: bool,
    pub(crate) on_ground: bool,
    pub(crate) layer: u16,
    pub(crate) sector: u16,
    pub(crate) box_ground: TraceSightObstacleBox,
    pub(crate) points: Vec<TraceSightObstaclePoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSightObstacleTypes {
    pub(crate) solid: bool,
    pub(crate) opaque: bool,
    pub(crate) projection_area: bool,
    pub(crate) mouse: bool,
    pub(crate) shield: bool,
    pub(crate) show_shadow_polygon: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSightObstacleBox {
    pub(crate) min: TracePoint,
    pub(crate) max: TracePoint,
}

#[derive(Clone, Debug, Deserialize, Serialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct TraceSightObstaclePoint {
    pub(crate) x: TraceFloat,
    pub(crate) y: TraceFloat,
    pub(crate) z_top: TraceFloat,
    pub(crate) z_bottom: TraceFloat,
}

#[cfg(test)]
mod nullable_field_tests {
    use super::{TraceActor, TraceAi, TraceSequenceElement, bitcode};
    use serde_json::{Value, json};

    #[test]
    fn historical_nullable_diagnostics_preserve_missing_null_and_populated_fields() {
        let actor = json!({
            "action_state": 1, "animation": 2, "command": 3,
            "command_name": "pass_door", "motion_state": 4, "wait_time": 5,
            "active_pass_door": {"gate_id": 51, "direct": true, "direction": -1},
            "sequence_element": {
                "id": 7, "type": 4, "state": 2, "command_level": 1,
                "command": 3, "command_name": "pass_door", "order_count": 1,
                "priority": 8, "posture_after_transition": 1,
                "action_state_after_transition": 2,
                "movement": {"action": 12, "pass_door": null},
                "following": {"id": 8}, "postponed": [1, 2],
                "current_order": "wait", "movement_payload": false
            }
        });
        for field in ["active_pass_door", "sequence_element"] {
            let decoded: TraceActor = serde_json::from_value(actor.clone()).unwrap();
            assert_eq!(serde_json::to_value(&decoded).unwrap()[field], actor[field]);
            let cached: TraceActor = bitcode::decode(&bitcode::encode(&decoded)).unwrap();
            assert_eq!(serde_json::to_value(cached).unwrap()[field], actor[field]);
            for missing in [false, true] {
                let mut historical = actor.clone();
                if missing {
                    historical.as_object_mut().unwrap().remove(field);
                } else {
                    historical[field] = Value::Null;
                }
                let decoded: TraceActor = serde_json::from_value(historical).unwrap();
                assert!(serde_json::to_value(decoded).unwrap()[field].is_null());
            }
        }
        let sequence = &actor["sequence_element"];
        for field in [
            "movement",
            "following",
            "postponed",
            "current_order",
            "movement_payload",
        ] {
            let decoded: TraceSequenceElement = serde_json::from_value(sequence.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(&decoded).unwrap()[field],
                sequence[field]
            );
            let cached: TraceSequenceElement = bitcode::decode(&bitcode::encode(&decoded)).unwrap();
            assert_eq!(
                serde_json::to_value(cached).unwrap()[field],
                sequence[field]
            );
            for missing in [false, true] {
                let mut historical = sequence.clone();
                if missing {
                    historical.as_object_mut().unwrap().remove(field);
                } else {
                    historical[field] = Value::Null;
                }
                let decoded: TraceSequenceElement = serde_json::from_value(historical).unwrap();
                assert!(serde_json::to_value(decoded).unwrap()[field].is_null());
            }
        }
        let ai = json!({
            "state": 1, "substate": 2, "macro_cursor": 65535,
            "my_line_jump": {
                "a": {"x": {"bits": 0}, "y": {"bits": 0}},
                "b": {"x": {"bits": 1065353216}, "y": {"bits": 0}}
            }
        });
        for field in ["macro_cursor", "my_line_jump"] {
            let decoded: TraceAi = serde_json::from_value(ai.clone()).unwrap();
            assert_eq!(serde_json::to_value(&decoded).unwrap()[field], ai[field]);
            let cached: TraceAi = bitcode::decode(&bitcode::encode(&decoded)).unwrap();
            assert_eq!(serde_json::to_value(cached).unwrap()[field], ai[field]);
            for missing in [false, true] {
                let mut historical = ai.clone();
                if missing {
                    historical.as_object_mut().unwrap().remove(field);
                } else {
                    historical[field] = Value::Null;
                }
                let decoded: TraceAi = serde_json::from_value(historical).unwrap();
                assert!(serde_json::to_value(decoded).unwrap()[field].is_null());
            }
        }
    }

    #[test]
    fn nullable_trace_fields_retain_typed_value_validation() {
        assert!(
            serde_json::from_value::<TraceAi>(json!({
                "state": 1, "substate": 2, "macro_cursor": 65536
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<TraceAi>(json!({
                "state": 1, "substate": 2, "my_line_jump": {"a": null, "b": null}
            }))
            .is_err()
        );
    }
}
