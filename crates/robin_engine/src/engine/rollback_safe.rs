//! Cross-crate `Engine` facade that makes the
//! "mutations-only-inside-the-tick" invariant mechanical.
//!
//! Downstream crates only ever see the [`Engine`] wrapper defined here.
//! It gives read-only access to the underlying [`EngineInner`] via
//! `Deref`; there is no `DerefMut` and no accessor returning
//! `&mut EngineInner`, so the only way to mutate simulation state from
//! outside `robin_engine` is through an explicit method on this type.
//!
//! Each exposed mutator is either:
//!
//! * a tick call (`apply_command(sim, s)`, `perform_hourglass`) — the normal
//!   per-frame sim-state mutation point,
//! * a one-shot setup / level-load / lifecycle hook, or
//! * a drain of a side-effect queue filled during the tick and consumed
//!   host-side.
//!
//! Anything that doesn't fit one of those buckets should be pushed into
//! the sim via `PlayerCommand` / a dedicated tick path, not added here.

use std::ops::Deref;

use super::SimConfig;
use super::{
    ConsoleResponse, DevState, DirectorCompletion, EngineError, EngineInner, InputState,
    LevelAssets, LevelLoadStaging, SideEffects, SimulationRng,
};
use crate::campaign::Campaign;
use crate::element::EntityId;
use crate::minimap::HitMask;
use crate::player_command::{PlayerCommand, PlayerInput};

/// Canonical gameplay-authoritative engine scalars emitted by schema-13
/// Original parity traces. Presentation camera/surface/backend state is
/// deliberately absent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ParityEngineState {
    pub cheat_used_flags: u32,
    pub lock_engine: bool,
    pub freeze_all: bool,
    pub locker: bool,
    pub speed: f32,
    pub speed_int: u16,
    pub mission_won: bool,
    pub mission_won_first_time: bool,
    pub quit_won: bool,
    pub quit_lost: bool,
    pub quit_interrupted: bool,
    pub script_globals: Vec<i32>,
}

/// Parallel runtime array whose length must match the loaded level geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotGridComponent {
    Lines,
    Sectors,
    Masks,
}

impl std::fmt::Display for SnapshotGridComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lines => "lines",
            Self::Sectors => "sectors",
            Self::Masks => "masks",
        })
    }
}

/// A decoded snapshot is incompatible with the already-loaded mission.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotRestoreError {
    #[error(
        "snapshot fast-grid {component} runtime length {snapshot_len} does not match loaded level length {level_len}"
    )]
    FastGridLengthMismatch {
        component: SnapshotGridComponent,
        snapshot_len: usize,
        level_len: usize,
    },
    #[error("snapshot world invariant failed: {detail}")]
    WorldInvariantViolation { detail: String },
    #[error("snapshot order invariant failed: {detail}")]
    OrderInvariantViolation { detail: String },
    #[error("snapshot level attachment failed: {detail}")]
    AttachmentFailure { detail: String },
}

/// Cross-crate owner of the simulation engine.
///
/// Downstream crates get `&EngineInner` via `Deref` and may only mutate
/// through the methods below.  There is no `DerefMut`, no accessor
/// returning `&mut EngineInner`, and `EngineInner::new` is
/// `pub(crate)`, so no alternative construction path leaks out either.
///
/// Internally (inside `robin_engine`) code still uses `EngineInner`
/// directly — the safety invariant is between the crate and its
/// downstream consumers, not a per-module check.
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
pub struct Engine {
    inner: EngineInner,
}

/// Level-load parameters for [`EngineArgs::level`].
///
/// The host is responsible for pre-loading the mission binaries
/// ([`crate::engine::level_loading::load_mission_for_campaign`]) and
/// pre-decoding the background bitmap (via the host-side
/// `pre_decode_background_map` helper) *before* calling
/// [`Engine::new`].  This lets the constructor size the grid
/// (`set_level_size`), ingest motion data, and run AI init with a
/// fully-populated `fast_grid` — instead of the previous split-init
/// pattern where `map_bbox` was zero and every patrol path failed
/// `TestIfPathIsFine`.
pub struct LevelLoadArgs<'a> {
    pub assets: &'a mut LevelAssets,
    pub level_directory: &'a str,
    pub progress: &'a mut dyn FnMut(f32),
    /// Pre-parsed mission + proto-level binaries.  See
    /// [`crate::engine::level_loading::load_mission_for_campaign`].
    pub loaded: crate::level_data::LoadedLevel,
    /// Background bitmap pixel dimensions, derived from the host's
    /// pre-decoded `PreDecodedBackground`.  Drives
    /// `FastFindGrid::size_map` and `CameraState::set_level_size` —
    /// both need real dims so `is_position_authorized` /
    /// `TestIfPathIsFine` work during `init_ai`.
    pub bg_pixel_dims: (f32, f32),
}

/// Ground-mark sprite metadata: sprite half-diagonal (half-width,
/// half-height) in world pixels and per-frame `(w, h)` sizes, used to
/// build the marker's move-box and on-screen culling rectangle.
///
/// `per_frame_offsets` is the per-frame `(x_min, y_min)` of the opaque
/// region, recorded when each frame is auto-cropped against the
/// `0x07C0` colour key.  The on-screen test adds this to the sprite's
/// top-left before testing the AABB, so plumbing it keeps the cull
/// rectangle aligned with the opaque pixels instead of biased by the
/// transparent border of the uncropped surface.
///
/// Host pre-computes this from the `RHID_GROUND_FOCUS` resource in
/// DEFAULT.RES and hands it to [`Engine::new`] so the sim can place
/// destination markers during the very first tick.
#[derive(Default, Clone)]
pub struct GroundMarkSpriteData {
    pub half_w: f32,
    pub half_h: f32,
    pub frame_sizes: Vec<(u16, u16)>,
    pub per_frame_offsets: Vec<(i16, i16)>,
}

/// Minimap corner-button widget setup: corner-sprite dimensions plus
/// the pixel-level hit mask built from frame 1 of `RHMAP_CORNER`.
/// Engine uses the canonical director view for this legacy minimap
/// setup; local widget placement lives host-side.
pub struct MinimapWidgetSetup {
    pub corner_size: crate::coordinates::ScreenSize,
    pub button_hit_mask: Option<HitMask>,
}

/// Arguments for [`Engine::new`].
///
/// Every field is required: a live `Engine` is defined as
/// "fully initialised for mission play", so construction requires the
/// host to already have loaded mission binaries, pre-decoded the
/// background bitmap, and gathered HUD/widget sprite metadata.  Test
/// and save-restore code paths that want a bare engine construct
/// `EngineInner` directly (it stays `pub(crate)` for that internal
/// use) or pass a test-fixture level through this same path.
pub struct EngineArgs<'a> {
    pub campaign: Campaign,
    pub level: LevelLoadArgs<'a>,
    /// Sprite metadata for the destination-marker ground mark (read
    /// from `RHID_GROUND_FOCUS`).  Used by `add_mark` to offset the
    /// click position and by the per-frame animation tick.  `None`
    /// when the host didn't find the resource (leaves the marker
    /// disabled).
    pub ground_mark_sprite: Option<GroundMarkSpriteData>,
    /// Per-row frame counts for the titbit sprite table.  Indexed by
    /// `SpriteRow` discriminant.  Used by `TitbitManager::num_frames_for_row`
    /// during animation.  Host pre-computes from DEFAULT.RES.  Empty
    /// when the resource is absent.
    pub titbit_row_frame_counts: Vec<u16>,
    /// Initial RNG seed.  Applied as the *first* mutation inside
    /// `Engine::new`, before any setup that draws from the engine's
    /// PRNG (entity spawn, AI init, mission script `StartUp`).  In
    /// single-player this is `0` (the historical default); in
    /// multiplayer it's the host-negotiated `mp_mission_seed`; under
    /// `--replay` it's the recording's header seed.  Threading the
    /// final seed through the constructor — instead of restoring it
    /// post-`Engine::new` — guarantees the engine's frame-0 state is
    /// a deterministic function of `EngineArgs` alone, with no
    /// SP↔MP-host divergence from RNG-consuming work between the
    /// two restore points.
    pub rng_seed: u64,
    /// Optional raw libc `rand()` prefix for original-game parity tooling.
    /// Normal game, replay, save, and multiplayer construction must use `None`.
    pub original_rng_replay: Option<Vec<u32>>,
    /// Complete deterministic configuration captured before level setup.
    /// Keeping the existing [`SimConfig`] intact prevents construction,
    /// rollback, replay, and network adoption from rebuilding only a subset
    /// of gameplay-affecting options.
    pub sim_config: SimConfig,
}

impl Engine {
    /// Read-only schema-13 parity view of campaign state at a frame boundary.
    #[doc(hidden)]
    pub fn parity_campaign(&self) -> &Campaign {
        &self.inner.mission_domain.campaign
    }

    /// Read-only schema-13 parity view of gameplay-authoritative global state.
    #[doc(hidden)]
    pub fn parity_engine_state(&self) -> ParityEngineState {
        let mission = &self.inner.mission_domain.state;
        let seat = &self.inner.players.seats[0];
        ParityEngineState {
            cheat_used_flags: self.inner.mission_domain.cheat_used_flags,
            lock_engine: self.inner.control.simulation_gates.engine_locked(),
            freeze_all: self.inner.control.simulation_gates.actors_frozen(),
            locker: seat.locker_active,
            speed: self.inner.control.speed,
            speed_int: self.inner.control.speed_int,
            mission_won: mission.mission_won,
            mission_won_first_time: mission.mission_won_first_time,
            quit_won: mission.quit_won,
            quit_lost: mission.quit_lost,
            quit_interrupted: mission.quit_interrupted,
            script_globals: self.inner.scripts.globals.clone(),
        }
    }

    /// Canonical manager-insertion-ordered sequence state for schema-13
    /// Original parity. Runtime allocation IDs are deliberately replaced by
    /// `(sequence ordinal, element index)` references.
    #[doc(hidden)]
    pub fn parity_sequence_manager_state(&self) -> serde_json::Value {
        use crate::sequence::{Field, FieldValue, SequenceElementData};
        use serde_json::{Value, json};

        let float = |value: f32| json!({ "bits": value.to_bits() });
        let point = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                crate::element::EntityIdKind::Soldier => "soldier",
                crate::element::EntityIdKind::Civilian => "civilian",
                crate::element::EntityIdKind::Fx => "fx",
                crate::element::EntityIdKind::Target => "target",
                crate::element::EntityIdKind::Bonus => "bonus",
                crate::element::EntityIdKind::Scroll => "scroll",
                crate::element::EntityIdKind::Projectile => "projectile",
                crate::element::EntityIdKind::Net => "net",
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let doors = &self.inner.script_domains.interactables.doors;
        let gate = |id: Option<crate::gate::DoorIndex>| -> Value {
            let Some(id) = id else { return Value::Null };
            let door = doors
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing door {id}"));
            let kind = match door.gate_type {
                crate::gate::GateType::Door => "door",
                crate::gate::GateType::Jump => "jump",
                crate::gate::GateType::None => "gate",
            };
            json!({
                "kind": kind,
                "sector_out": door.sector_out.get(),
                "sector_in": door.sector_in.get(),
                "layer_out": door.layer_out,
                "layer_in": door.layer_in,
                "point_out": point(door.point_out.x, door.point_out.y),
                "point_in": point(door.point_in.x, door.point_in.y),
            })
        };
        let lines = &self.inner.world.fast_grid.level.jump_lines;
        let line = |id: Option<crate::jump_line::JumpLineIndex>| -> Value {
            let Some(id) = id else { return Value::Null };
            let line = lines
                .get(usize::from(id))
                .unwrap_or_else(|| panic!("parity sequence references missing jump line {id}"));
            json!({
                "a": point(line.point_a.x, line.point_a.y),
                "b": point(line.point_b.x, line.point_b.y),
            })
        };

        let manager = &self.inner.orders.sequence_manager;
        let sequence_ordinals: std::collections::BTreeMap<_, _> = manager
            .sequences_iter()
            .enumerate()
            .map(|(ordinal, sequence)| (sequence.id, ordinal))
            .collect();
        let reference = |id: crate::sequence::SequenceId, element: usize| {
            let sequence = sequence_ordinals.get(&id).copied().unwrap_or_else(|| {
                panic!("parity sequence reference points outside manager: {id:?}/{element}")
            });
            json!({ "sequence": sequence, "element": element })
        };

        let mut sequences = Vec::new();
        for sequence in manager.sequences_iter() {
            let (cursor, current_level, running, in_progress, started) = sequence.parity_counters();
            let mut elements = Vec::new();
            for element_state in &sequence.elements {
                let orders: Vec<_> = element_state
                    .orders
                    .iter()
                    .map(|order| {
                        json!({
                            "action": order.order_type as u32,
                            "destination": point(order.target_x, order.target_y),
                            "tolerance": float(order.tolerance),
                            "reverse": order.reverse,
                            "compute_direction": order.compute_direction,
                            "done": order.done,
                            "antagonist": order.antagonist.map(&entity).unwrap_or(Value::Null),
                        })
                    })
                    .collect();

                let subtype = match &element_state.data {
                    SequenceElementData::Simple => json!({ "kind": "simple" }),
                    SequenceElementData::Interaction { antagonist } => json!({
                        "kind": "interaction",
                        "antagonist": antagonist.map(&entity).unwrap_or(Value::Null),
                    }),
                    SequenceElementData::Damage {
                        origin,
                        projectile,
                        damage,
                        concussion,
                        sword_strike,
                        is_harder_hit,
                        ..
                    } => json!({
                        "kind": "damage",
                        "origin": origin.map(&entity).unwrap_or(Value::Null),
                        "damage": damage,
                        "concussion": concussion,
                        "harder_hit": is_harder_hit,
                        "sword_strike": sword_strike.map(|strike| strike as i32).unwrap_or(11),
                        "arrow": projectile.map(&entity).unwrap_or(Value::Null),
                    }),
                    SequenceElementData::Movement {
                        destination,
                        layer,
                        sector,
                        gate_id,
                        line_id,
                        element,
                        flags,
                        tolerance,
                        direction,
                        action,
                        speed_factor,
                        ..
                    } => {
                        let linked_seek = element_state
                            .legacy_v48
                            .as_ref()
                            .and_then(|legacy| legacy.linked_seek)
                            .flatten()
                            .map(|linked| reference(linked.sequence_id, linked.element_index))
                            .unwrap_or(Value::Null);
                        json!({
                            "kind": "movement",
                            "destination": point(destination.x, destination.y),
                            "layer": layer,
                            "sector": sector.map(|value| value.get() as i32).unwrap_or(-1),
                            "gate": gate(*gate_id),
                            "line": line(*line_id),
                            "target": element.map(&entity).unwrap_or(Value::Null),
                            "flags": flags.bits(),
                            "tolerance": float(*tolerance),
                            "direction": direction,
                            "action": *action as u32,
                            "speed_factor": float(*speed_factor),
                            "linked_seek": linked_seek,
                        })
                    }
                    SequenceElementData::Generic { properties } => {
                        let mut ordered: Vec<_> = properties
                            .iter()
                            .filter_map(|(field, value)| {
                                field
                                    .original_ordinal()
                                    .map(|ordinal| (ordinal, *field, value))
                            })
                            .collect();
                        ordered.sort_by_key(|(ordinal, _, _)| *ordinal);
                        let properties: Vec<_> = ordered
                            .into_iter()
                            .map(|(ordinal, field, value)| {
                                let value = match value {
                                    FieldValue::Bool(value) => json!(value),
                                    FieldValue::Integer(value) => {
                                        if matches!(
                                            field,
                                            Field::JumplineSource | Field::JumplineDestination
                                        ) && *value == 0
                                        {
                                            Value::Null
                                        } else {
                                            json!(value)
                                        }
                                    }
                                    FieldValue::Float(value) => float(*value),
                                    FieldValue::GeoPoint2D { x, y } => point(*x, *y),
                                    FieldValue::Point3D { x, y, z } => point3(*x, *y, *z),
                                    FieldValue::Element(value) => entity(*value),
                                    FieldValue::OptionalElement(value) => {
                                        value.map(&entity).unwrap_or(Value::Null)
                                    }
                                    FieldValue::Animation(value) => json!(*value as u32),
                                    FieldValue::LineId(value) => line(Some(*value)),
                                    FieldValue::OptionalLineId(value) => line(*value),
                                    FieldValue::DoorId(value) => gate(Some(*value)),
                                    FieldValue::OptionalDoorId(value) => gate(*value),
                                };
                                json!({ "field": ordinal, "value": value })
                            })
                            .collect();
                        json!({ "kind": "generic", "properties": properties })
                    }
                };

                let postponed = match (
                    element_state.postponed_element_index,
                    element_state.cross_postponed,
                ) {
                    (Some(index), None) => reference(sequence.id, index),
                    (None, Some((id, index))) => reference(id, index),
                    (None, None) => Value::Null,
                    (Some(_), Some(_)) => panic!(
                        "parity sequence element carries both intra- and cross-sequence postponed refs"
                    ),
                };
                let transition_live =
                    element_state.state == crate::sequence::SequenceState::InProgress;
                elements.push(json!({
                    "command": element_state.command as u32,
                    "level": element_state.command_level,
                    "owner": element_state.owner.map(&entity).unwrap_or(Value::Null),
                    "state": element_state.state as u32,
                    "priority": element_state.priority as u32,
                    "posture_after_transition": transition_live
                        .then(|| json!(element_state.posture_after_transition as u32))
                        .unwrap_or(Value::Null),
                    "action_state_after_transition": transition_live
                        .then(|| json!(element_state.action_state_after_transition as u32))
                        .unwrap_or(Value::Null),
                    "transition_orders": transition_live
                        .then(|| json!(element_state.num_transition_orders))
                        .unwrap_or(Value::Null),
                    "script_driven": element_state.script_driven,
                    "postponed": postponed,
                    "orders": orders,
                    "subtype": subtype,
                }));
            }
            sequences.push(json!({
                "cursor": cursor,
                "current_level": current_level,
                "running_elements": running,
                "elements_in_progress": in_progress,
                "started": started,
                "elements": elements,
            }));
        }

        let (elements_to_go, actor_current) = manager.parity_runtime_refs();
        json!({
            "sequences": sequences,
            "elements_to_go": elements_to_go
                .into_iter()
                .map(|(id, element)| reference(id, element))
                .collect::<Vec<_>>(),
            "actor_current": actor_current
                .into_iter()
                .map(|(owner, selected)| json!({
                    "owner": entity(owner),
                    "element": reference(selected.sequence_id, selected.element_index),
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// Persistent Original-compatible pathfinder state at a stable frame
    /// boundary. Synchronous A* scratch is deliberately absent; a computed
    /// READY head remains authoritative until the next scheduling barrier.
    #[doc(hidden)]
    pub fn parity_pathfinder_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let float = |value: f32| json!({ "bits": value.to_bits() });
        let point = |point: crate::coordinates::MapPoint| json!({ "x": float(point.x), "y": float(point.y) });
        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                crate::element::EntityIdKind::Soldier => "soldier",
                crate::element::EntityIdKind::Civilian => "civilian",
                crate::element::EntityIdKind::Fx => "fx",
                crate::element::EntityIdKind::Target => "target",
                crate::element::EntityIdKind::Bonus => "bonus",
                crate::element::EntityIdKind::Scroll => "scroll",
                crate::element::EntityIdKind::Projectile => "projectile",
                crate::element::EntityIdKind::Net => "net",
            };
            json!({ "kind": kind, "index": id.index() })
        };

        let manager = &self.inner.orders.sequence_manager;
        let sequence_ordinals: std::collections::BTreeMap<_, _> = manager
            .sequences_iter()
            .enumerate()
            .map(|(ordinal, sequence)| (sequence.id, ordinal))
            .collect();
        let reference = |id: crate::sequence::SequenceId, element: usize| {
            let sequence = sequence_ordinals.get(&id).copied().unwrap_or_else(|| {
                panic!("pathfinder request references unmanaged sequence {id:?}/{element}")
            });
            json!({ "sequence": sequence, "element": element })
        };

        let (ignore_next_path, pending) = self
            .inner
            .orders
            .pending_path_requests
            .parity_state(&self.inner.world.fast_grid);
        let ready = pending.first().is_some_and(|entry| entry.in_flight);
        if pending.iter().skip(1).any(|entry| entry.in_flight) {
            panic!("pathfinder snapshot contains more than one in-flight request");
        }
        let requests = pending
            .into_iter()
            .map(|entry| {
                let request = entry.request;
                let waypoints = entry
                    .waypoints
                    .map(|points| Value::Array(points.into_iter().map(point).collect()))
                    .unwrap_or(Value::Null);
                json!({
                    "request": {
                        "actor": entity(request.actor),
                        "antagonist": request.antagonist.map(&entity).unwrap_or(Value::Null),
                        "layer": request.layer,
                        "area": request.area,
                        "source": point(request.source),
                        "goal": point(request.goal),
                        "half_diagonal_index": request.half_diagonal_index,
                        "half_diagonal": {
                            "x": float(request.half_diagonal.x),
                            "y": float(request.half_diagonal.y),
                        },
                        "animation": request.animation,
                        "reverse": request.reverse,
                        "speed": request.speed,
                        "tolerance": float(request.tolerance),
                        "use_first_point": request.use_first_point,
                    },
                    "element": reference(entry.sequence_id, entry.element_index),
                    "in_flight": entry.in_flight,
                    "waypoints": waypoints,
                })
            })
            .collect::<Vec<_>>();

        json!({
            "status": if ready { "ready" } else { "waiting" },
            "ignore_next_path": ignore_next_path,
            "number_of_attempts": self.inner.world.pathfinder.number_of_attempts,
            "area_states": &self.inner.world.pathfinder.states,
            "requests": requests,
        })
    }

    /// Current-frame, behaviorally reusable `ComputeViewRadius` entries.
    /// Stale entries and zero-radius writes are omitted because Original's
    /// getter treats both as misses. Projection surfaces use their ordinal in
    /// Original's projection-area array after the synthetic ground slot.
    #[doc(hidden)]
    pub fn parity_view_radius_cache_state(&self, assets: &LevelAssets) -> serde_json::Value {
        use serde_json::json;

        let frame = self.inner.control.frame_counter;
        let cache = &self.inner.ai.view_radius_cache;
        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                crate::element::EntityIdKind::Soldier => "soldier",
                crate::element::EntityIdKind::Civilian => "civilian",
                crate::element::EntityIdKind::Fx => "fx",
                crate::element::EntityIdKind::Target => "target",
                crate::element::EntityIdKind::Bonus => "bonus",
                crate::element::EntityIdKind::Scroll => "scroll",
                crate::element::EntityIdKind::Projectile => "projectile",
                crate::element::EntityIdKind::Net => "net",
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let entry = |surface: serde_json::Value, value: crate::ai_vision::ViewRadiusCacheEntry| {
            assert!(
                value.radius.is_finite(),
                "view-radius cache contains non-finite radius for {:?}",
                value.viewer
            );
            json!({
                "surface": surface,
                "viewer": entity(value.viewer),
                "radius": { "bits": value.radius.to_bits(), "value": value.radius },
            })
        };

        let mut entries = Vec::new();
        if let Some(value) = cache.ground
            && value.frame == frame
            && value.radius != 0.0
        {
            entries.push(entry(json!({ "kind": "ground" }), value));
        }

        let obstacles = self.inner.sight_obstacles(assets);
        let mut projection_ordinal = 0usize;
        for (raw_index, obstacle) in obstacles.iter_indexed() {
            if !obstacle.is_projection_area() {
                continue;
            }
            let ordinal = projection_ordinal;
            projection_ordinal += 1;
            let Some(value) = cache.obstacles.get(raw_index as usize).copied().flatten() else {
                continue;
            };
            if value.frame != frame || value.radius == 0.0 {
                continue;
            }
            entries.push(entry(
                json!({ "kind": "projection", "index": ordinal }),
                value,
            ));
        }
        for (raw_index, value) in cache.obstacles.iter().enumerate() {
            if value.is_some() && raw_index >= obstacles.len() {
                panic!("view-radius cache references missing obstacle {raw_index}");
            }
            if let Some(value) = value
                && value.frame == frame
                && value.radius != 0.0
                && !obstacles
                    .get(raw_index)
                    .expect("validated obstacle index")
                    .is_projection_area()
            {
                panic!("view-radius cache references non-projection obstacle {raw_index}");
            }
        }

        json!({ "frame": frame, "entries": entries })
    }

    /// Persistent mission-VM state at a quiescent frame boundary. VM native
    /// handles are decoded to their semantic table kind/index; raw process or
    /// Rust handle encodings never enter the trace.
    #[doc(hidden)]
    pub fn parity_script_runtime_state(&self) -> serde_json::Value {
        use crate::scb::TypeTag;
        use crate::script_manager::ScriptInstance;
        use serde_json::{Value, json};

        let Some(script) = self.inner.scripts.mission.as_ref() else {
            return json!({
                "static_words": [], "instances": [], "computed_locations": []
            });
        };
        script.assert_no_active_call_frames();
        if script.state.sequence_recorder.recording.is_some() {
            panic!("parity script capture reached an open sequence recording");
        }

        let float = |value: f32| json!({ "bits": value.to_bits() });
        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                crate::element::EntityIdKind::Soldier => "soldier",
                crate::element::EntityIdKind::Civilian => "civilian",
                crate::element::EntityIdKind::Fx => "fx",
                crate::element::EntityIdKind::Target => "target",
                crate::element::EntityIdKind::Bonus => "bonus",
                crate::element::EntityIdKind::Scroll => "scroll",
                crate::element::EntityIdKind::Projectile => "projectile",
                crate::element::EntityIdKind::Net => "net",
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let actor_entity = |handle: i32| {
            let index = crate::natives::ScriptHandleCodec::actor_handle_index(handle)
                .unwrap_or_else(|| panic!("script instance has invalid actor handle {handle}"));
            let id = self
                .inner
                .world
                .entities
                .id_at_legacy_slot(index as u32)
                .unwrap_or_else(|| panic!("script instance actor slot {index} is empty"));
            entity(id)
        };

        let native = |type_name: &str, bits: u32| -> Value {
            if bits == 0 {
                return Value::Null;
            }
            let handle = bits as i32;
            if matches!(type_name, "Actor" | "Scroll") {
                let index = crate::natives::ScriptHandleCodec::actor_handle_index(handle)
                    .unwrap_or_else(|| {
                        panic!("parity script member {type_name} has invalid handle 0x{bits:08x}")
                    });
                let id = self
                    .inner
                    .world
                    .entities
                    .id_at_legacy_slot(index as u32)
                    .unwrap_or_else(|| panic!("script member {type_name} slot {index} is empty"));
                if type_name == "Actor"
                    && let Some(mobile) =
                        self.inner
                            .world
                            .entities
                            .get(id)
                            .and_then(|entity| match entity {
                                crate::element::Entity::Fx(fx) => fx.fx.mobile_index,
                                _ => None,
                            })
                {
                    return json!({ "kind": type_name, "mobile": mobile });
                }
                return json!({ "kind": type_name, "entity": entity(id) });
            }
            let index = match type_name {
                "Door" => crate::natives::ScriptHandleCodec::door_index(handle),
                "Patch" => crate::natives::ScriptHandleCodec::patch_index(handle),
                "Location" => crate::natives::ScriptHandleCodec::location_index(handle),
                "SoundSource" => crate::natives::ScriptHandleCodec::sound_source_index(handle),
                "Building" => crate::natives::ScriptHandleCodec::building_index(handle),
                "Way" => crate::natives::ScriptHandleCodec::way_index(handle),
                _ => panic!("parity script state encountered unknown native type {type_name}"),
            }
            .unwrap_or_else(|| {
                panic!("parity script member {type_name} has invalid handle 0x{bits:08x}")
            });
            json!({ "kind": type_name, "index": index })
        };

        let vm_state = |instance: &ScriptInstance| {
            let class = script
                .manager
                .scb()
                .classes
                .get(instance.class_idx())
                .unwrap_or_else(|| {
                    panic!(
                        "script instance has invalid class index {}",
                        instance.class_idx()
                    )
                });
            let members: Vec<_> = class
                .member_variables
                .iter()
                .map(|member| {
                    let address = usize::try_from(member.address).unwrap_or_else(|_| {
                        panic!("script member {} has negative heap address", member.name)
                    });
                    let bytes = instance
                        .vm
                        .heap
                        .get(address..address + 4)
                        .unwrap_or_else(|| panic!("script member {} exceeds VM heap", member.name));
                    let bits = u32::from_le_bytes(bytes.try_into().expect("four-byte VM word"));
                    let type_name = match member.ty.tag {
                        TypeTag::Bool => "bool",
                        TypeTag::Int => "int",
                        TypeTag::Float => "float",
                        TypeTag::Void => "void",
                        TypeTag::NativeType => member.ty.native_type_name.as_str(),
                        TypeTag::NotDefined
                        | TypeTag::Event
                        | TypeTag::Function
                        | TypeTag::NativeFunction => "NotExpected",
                    };
                    let value = if member.ty.tag == TypeTag::NativeType {
                        native(type_name, bits)
                    } else {
                        json!({ "bits": bits })
                    };
                    json!({ "name": member.name, "type": type_name, "value": value })
                })
                .collect();
            json!({ "class": class.class_name, "members": members })
        };

        let mut static_words = Vec::new();
        let chunks = script.manager.static_area.chunks_exact(4);
        if !chunks.remainder().is_empty() {
            panic!("parity script static area is not word aligned");
        }
        for (word, bytes) in chunks.enumerate() {
            let bits = u32::from_le_bytes(bytes.try_into().expect("four-byte static word"));
            if bits != 0 {
                static_words.push(json!({ "offset": word * 4, "bits": bits }));
            }
        }

        let mut instances = Vec::new();
        instances.push(json!({
            "owner": { "kind": "engine" },
            "vm": vm_state(&script.instance),
        }));

        let mut entity_instances: Vec<(u32, &str, EntityId, &ScriptInstance)> = Vec::new();
        for (&handle, instance) in &script.actor_instances {
            let owner = actor_entity(handle);
            let index = owner["index"].as_u64().expect("entity index") as u32;
            let id = self
                .inner
                .world
                .entities
                .id_at_legacy_slot(index)
                .expect("actor entity");
            entity_instances.push((index, "actor", id, instance));
        }
        for (&handle, instance) in &script.target_instances {
            let owner = actor_entity(handle);
            let index = owner["index"].as_u64().expect("entity index") as u32;
            let id = self
                .inner
                .world
                .entities
                .id_at_legacy_slot(index)
                .expect("target entity");
            entity_instances.push((index, "target", id, instance));
        }
        for (&handle, instance) in &script.scroll_instances {
            let owner = actor_entity(handle);
            let index = owner["index"].as_u64().expect("entity index") as u32;
            let id = self
                .inner
                .world
                .entities
                .id_at_legacy_slot(index)
                .expect("scroll entity");
            entity_instances.push((index, "scroll", id, instance));
        }
        entity_instances.sort_by_key(|(index, _, _, _)| *index);
        instances.extend(entity_instances.into_iter().map(|(_, kind, id, instance)| {
            json!({
                "owner": { "kind": kind, "entity": entity(id) },
                "vm": vm_state(instance),
            })
        }));

        for (&zone, instance) in &script.zone_instances {
            let location = script.bindings.script_point_count + zone;
            let grid_index = *script
                .bindings
                .script_zone_grid_indices
                .get(zone)
                .unwrap_or_else(|| panic!("script VM references missing zone {zone}"));
            let sector = self
                .inner
                .world
                .fast_grid
                .level
                .sectors
                .get(grid_index as usize)
                .unwrap_or_else(|| {
                    panic!("script zone {zone} has missing grid sector {grid_index}")
                })
                .sector_number
                .get();
            instances.push(json!({
                "owner": { "kind": "zone", "location": location, "sector": sector },
                "vm": vm_state(instance),
            }));
        }
        for (&(path, waypoint), instance) in &script.waypoint_instances {
            instances.push(json!({
                "owner": {
                    "kind": "waypoint", "path": path.get(), "waypoint": waypoint,
                },
                "vm": vm_state(instance),
            }));
        }

        let computed_locations: Vec<_> = script
            .state
            .computed_locations
            .iter()
            .map(|location| {
                let Some(location) = location else {
                    return Value::Null;
                };
                let (Some(layer), Some(sector)) = (location.layer, location.sector) else {
                    panic!("non-null computed script location lacks spatial attachment");
                };
                json!({
                    "position": {
                        "x": float(location.position.0), "y": float(location.position.1),
                    },
                    "layer": layer,
                    "sector": sector,
                })
            })
            .collect();

        json!({
            "static_words": static_words,
            "instances": instances,
            "computed_locations": computed_locations,
        })
    }

    /// Read-only schema-13 snapshot of the ordered failed-path timeout list.
    #[doc(hidden)]
    pub fn parity_failed_path_requests(&self) -> Vec<crate::pathfinder::ParityFailedPathRequest> {
        self.inner.parity_failed_path_requests()
    }

    /// Crate-internal access for the validated Original-save adoption
    /// coordinator. Downstream callers cannot bypass `Engine` construction or
    /// replace a partially converted mission.
    pub(crate) fn legacy_adoption_inner(&self) -> &EngineInner {
        &self.inner
    }

    /// Install a fully preflighted detached Original-save candidate.
    ///
    /// This remains crate-internal until every authoritative v48 section is
    /// represented by the coordinator.
    pub(crate) fn install_legacy_adoption_inner(&mut self, inner: EngineInner) {
        self.inner = inner;
    }

    /// Run pre-level campaign mission selection on the same authoritative
    /// stream that the selected mission will receive at construction.
    ///
    /// The original process uses one `rand()` sequence across campaign and
    /// mission code. This temporary bare engine owns that sequence while no
    /// loaded mission engine exists; the returned seed is the complete next
    /// RNG state and must be passed to [`EngineArgs::rng_seed`].
    pub fn select_next_mission(
        campaign: Campaign,
        profiles: &crate::profiles::ProfileManager,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> (Campaign, usize, u64, SimConfig) {
        let mut inner = EngineInner::new_with_campaign(campaign);
        inner.control.sim_config = sim_config;
        inner.restore_rng_from_seed(rng_seed);
        let mission_idx = inner.with_simulation_context(|inner, sim| {
            inner
                .mission_domain
                .required_campaign_mut("selecting the next mission")
                .determine_next_mission(sim, profiles)
        });
        let rng_seed = inner.rng_seed();
        let sim_config = inner.control.sim_config;
        (inner.into_campaign(), mission_idx, rng_seed, sim_config)
    }

    /// Create a fully-initialised engine for mission play.
    ///
    /// The host is expected to have:
    ///
    /// 1. Built `Campaign` + selected the current mission.
    /// 2. Loaded the mission binaries via
    ///    [`crate::engine::level_loading::load_mission_for_campaign`].
    /// 3. Pre-decoded the background bitmap (host-side helper) and
    ///    recorded its pixel dimensions.
    /// 4. Optionally pre-decoded the minimap bitmap.
    ///
    /// With those in hand, this constructor runs every step the old
    /// split `Engine::new` + `apply_level_bitmaps_loaded` pair used to
    /// do — `initialize_from_campaign` (entity spawn, mission script),
    /// `set_level_size`, the motion stage (pathfinder
    /// graph + grid sector registration), `initialize` (mission-script init
    /// followed by AI init, both of which now see a real `map_bbox` +
    /// half-diagonals table), and — for Sherwood —
    /// `apply_production_sector_data`.
    ///
    /// Returns `Err` only when mission data fails to ingest.
    pub fn new(args: EngineArgs) -> Result<Self, EngineError> {
        Self::new_preserving_campaign(args).map_err(|(error, _campaign)| error)
    }

    /// Append one original frame's raw RNG values to an active parity replay.
    pub fn append_original_rng_replay(&mut self, draws: Vec<u32>) {
        self.inner.control.rng.append_original_replay(draws);
    }

    /// Replace and rewind the raw Original RNG stream used by parity tools.
    ///
    /// Loaded saves restore a serialized engine and RNG seed after mission
    /// construction. A reconstruction tool may therefore need one copy of
    /// the seeded stream for fresh Rust construction, then rewind to the
    /// post-load stream boundary recorded by the Original.
    pub fn replace_original_rng_replay(&mut self, draws: Vec<u32>) {
        self.inner.control.rng.replace_original_replay(draws);
    }

    /// Number of original raw RNG values consumed so far, when parity replay is active.
    pub fn original_rng_replay_cursor(&self) -> Option<usize> {
        self.inner.control.rng.original_replay_cursor()
    }

    /// Rust RNG sites which consumed a selected interval of original draws.
    pub fn original_rng_replay_sites(
        &self,
        range: std::ops::Range<usize>,
    ) -> Option<Vec<crate::sim_rng::RngSite>> {
        self.inner.control.rng.original_replay_sites(range)
    }

    /// Clone the complete engine for structured diagnostics while omitting
    /// the Original parity replay capability, which intentionally cannot be
    /// serialized as an ordinary save/rollback snapshot.
    ///
    /// Diagnostic callers must record [`Self::original_rng_replay_cursor`]
    /// alongside the returned snapshot. All other engine state is unchanged.
    pub fn diagnostic_snapshot_without_original_rng_replay(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.inner.control.rng = self.inner.control.rng.clone_without_original_replay();
        snapshot
    }

    /// Create a fully-initialised engine while preserving ownership of the
    /// supplied campaign if mission ingestion fails.
    ///
    /// `EngineArgs` consumes its campaign. Callers which are themselves
    /// ownership boundaries (notably mission bootstrap) must use this variant
    /// so an initialization error cannot silently drop the one live campaign.
    /// [`Engine::new`] remains the convenience wrapper for callers which do
    /// not need to recover that value.
    pub fn new_preserving_campaign(
        args: EngineArgs,
    ) -> Result<Self, (EngineError, crate::campaign::Campaign)> {
        let mut inner = EngineInner::new_with_campaign(args.campaign);
        inner.control.sim_config = args.sim_config;
        inner.control.mission_start_rng_seed = args.rng_seed;
        inner.control.mission_start_sim_config = args.sim_config;
        // Seed the PRNG and apply engine-global cheat flags FIRST,
        // before any setup that might draw from the RNG or branch on
        // the cheat flag.  See `EngineArgs::rng_seed` /
        // `EngineArgs::sim_config` docs for the rationale.
        inner.restore_rng_from_seed(args.rng_seed);
        if let Some(draws) = args.original_rng_replay {
            inner.control.rng = SimulationRng::with_original_replay(draws);
        }
        inner.set_golden_eye_mode(args.sim_config.golden_eye);
        if let Some(gm) = args.ground_mark_sprite {
            inner.set_ground_mark_sprite_data(
                gm.half_w,
                gm.half_h,
                gm.frame_sizes,
                gm.per_frame_offsets,
            );
        }
        if !args.titbit_row_frame_counts.is_empty() {
            inner.set_titbit_row_frame_counts(args.titbit_row_frame_counts);
        }
        let LevelLoadArgs {
            assets,
            level_directory,
            progress,
            loaded,
            bg_pixel_dims,
        } = args.level;
        assets.entities.mobile_element_count = 0;
        assets.scripts.mission_name = None;
        // The proto-level (motion sectors) loads before the mission
        // file (beam-mes / soldiers / civilians).  We thread
        // `bg_pixel_dims` into `initialize_from_campaign`, which calls
        // `set_level_size` + the motion stage mid-load
        // (right after the proto data is stashed in constructor-local
        // pending data, but before any entity that references a sector
        // spawns) so that beam-me sector validation and downstream
        // sector-handle resolution see the populated grid.
        let mut staging = LevelLoadStaging::default();
        if let Err(error) = inner.with_simulation_context(|inner, sim| {
            inner.initialize_from_campaign(
                sim,
                assets,
                &mut staging,
                loaded,
                level_directory,
                bg_pixel_dims,
                progress,
            )
        }) {
            let campaign = inner.into_campaign();
            return Err((error, campaign));
        }
        inner.populate_sector_gates_from_doors();
        let original_topology =
            match crate::legacy_save::topology_adapter::derive_static_element_topology(
                &inner, assets,
            ) {
                Ok(topology) => topology,
                Err(error) => {
                    let campaign = inner.into_campaign();
                    return Err((
                        EngineError::MissionLevelStage {
                            stage: "Original element identity",
                            reason: error.to_string(),
                        },
                        campaign,
                    ));
                }
            };
        inner.world.install_original_creation_orders(
            original_topology.creation_order_by_entity,
            original_topology.static_creation_order_boundary,
        );
        // Mission-script init and then AI init run HERE — after pathfinder +
        // grid are fully populated. This preserves RHEngine::Initialize's
        // script-before-AI order while still letting TestIfPathIsFine /
        // is_position_authorized see the real map and motion lines.
        inner.initialize(assets);
        assets.level_grid = inner.world.fast_grid.level.clone();
        assets.entities.mobile_element_count = inner.world.mobile_elements.len();
        assets.scripts.mission_name = inner
            .scripts
            .mission
            .as_ref()
            .map(|script| script.script_name.clone());

        // Sherwood-only: spawn production bonuses at the registered
        // points.
        let campaign = inner.campaign();
        let is_sherwood = campaign.current_mission_idx.is_some_and(|i| {
            campaign.missions[i]
                .profile(&assets.profile_manager)
                .location
                == crate::profiles::MissionLocation::Sherwood
        });
        if is_sherwood {
            inner.with_simulation_context(|inner, sim| {
                inner.apply_production_sector_data(sim, assets);
                // Fire the "production-sector data is ready" hook
                // (`SendMessage(0, 1001)`) the Sherwood StartUp script
                // listens for on fresh Sherwood entry.  The LevelLoad twin
                // is handled via the post-load fixup path; this arm covers
                // fresh entry only.
                inner.dispatch_startup_message(sim, assets, 1001, 0, 0);
            });
        }
        inner
            .world
            .validate_level_attachments(assets, inner.script_domains.zones.scripts.len());
        Ok(Self { inner })
    }

    /// Test-only shortcut: build an `Engine` with an empty fixture
    /// level.  Equivalent to the old
    /// `Engine::new(EngineArgs { ..Default::default() })` spelling
    /// that disappeared when `Engine::new` went RAII.
    ///
    /// Used from unit tests that want an engine for serde round-trip,
    /// command-pipeline, or HUD testing without loading a real
    /// mission from disk.  Not suitable for anything that touches the
    /// pathfinder, motion grid, or AI — the fixture level has no
    /// entities, no motion data, and no pathfinder graph.
    pub fn new_for_test(
        screen_width: f32,
        screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            screen_width,
            screen_height,
            campaign,
            assets,
            0.0,
            0.0,
            0,
            SimConfig::default(),
        )
    }

    /// Variant of [`Engine::new_for_test`] that lets the caller set
    /// non-zero map dimensions — needed by tests that touch the
    /// cutscene camera's zoom / scroll clamps, which key off `level_size`.
    pub fn new_for_test_with_level_size(
        _screen_width: f32,
        _screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        map_width: f32,
        map_height: f32,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            _screen_width,
            _screen_height,
            campaign,
            assets,
            map_width,
            map_height,
            0,
            SimConfig::default(),
        )
    }

    /// Test fixture variant that supplies the exact mission-construction
    /// seed/config used by replay, save preflight, and multiplayer tests.
    pub fn new_for_test_with_simulation(
        screen_width: f32,
        screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> Result<Self, super::EngineError> {
        Self::new_for_test_with_level_size_and_simulation(
            screen_width,
            screen_height,
            campaign,
            assets,
            0.0,
            0.0,
            rng_seed,
            sim_config,
        )
    }

    fn new_for_test_with_level_size_and_simulation(
        _screen_width: f32,
        _screen_height: f32,
        campaign: Campaign,
        assets: &mut LevelAssets,
        map_width: f32,
        map_height: f32,
        rng_seed: u64,
        sim_config: SimConfig,
    ) -> Result<Self, super::EngineError> {
        use crate::mission::Mission;
        use crate::profiles::MissionProfile;

        let mut campaign = campaign;

        // `initialize_from_campaign` expects `current_mission_idx`,
        // `campaign.missions[idx]`, and `profiles.missions[profile_idx]`
        // all to resolve.  When the caller hasn't populated any of
        // those (the common test case), plant a minimal fixture entry
        // at index 0.  We mutate `assets.profile_manager` via
        // `Arc::make_mut` so callers that share the same profiles Arc
        // pick up the fixture.
        if assets.profile_manager.missions.is_empty() {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.missions.push(MissionProfile::default());
        }
        if campaign.missions.is_empty() {
            campaign.missions.push(Mission {
                profile_idx: Some(0),
                ..Mission::default()
            });
        }
        if campaign.current_mission_idx.is_none() {
            campaign.current_mission_idx = Some(0);
        }

        let loaded = crate::level_data::LoadedLevel::empty_for_test();
        Self::new(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (map_width, map_height),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed,
            original_rng_replay: None,
            sim_config,
        })
    }

    // ── Tick ────────────────────────────────────────────────────────

    /// Select whether recorded between-frame director events own completion
    /// timing for camera sequence elements.
    pub fn set_external_director_completion_replay(&mut self, enabled: bool) {
        self.inner.set_external_director_completion_replay(enabled);
    }

    /// Queue concrete speech samples resolved by the between-frame logical
    /// sound-manager update. They are consumed by the next engine tick.
    pub fn queue_resolved_exclamations(
        &mut self,
        resolutions: Vec<crate::sound::ResolvedExclamation>,
    ) {
        self.inner.queue_resolved_exclamations(resolutions);
    }

    /// Apply one recorded director completion at the pre-Hourglass boundary.
    ///
    /// This validates the currently latched sequence command, terminates it,
    /// and synchronously runs immediate successors before returning.
    pub fn apply_external_director_completion(
        &mut self,
        completion: DirectorCompletion,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
    ) -> Result<(), String> {
        self.require_live_campaign("applying an external director completion");
        self.inner
            .apply_external_director_completion(completion, display, assets)
    }

    /// The per-frame simulation tick. The ONLY per-frame sim-state
    /// mutation point; rollback replay re-runs this on a cloned engine
    /// and must see bit-identical results.
    pub fn perform_hourglass(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> SideEffects {
        self.require_live_campaign("performing an engine tick");
        self.inner.perform_hourglass(display, assets, dev)
    }

    /// Run a simulation tick while optionally forcing the actor/world body
    /// gate closed after the mission-script phase.
    ///
    /// This does not alter the engine's persistent lock state.
    pub fn perform_hourglass_with_body_gate(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
        simulation_body_allowed: bool,
    ) -> SideEffects {
        self.require_live_campaign("performing a gated engine tick");
        self.inner
            .perform_hourglass_with_body_gate(display, assets, dev, simulation_body_allowed)
    }

    /// One-shot lifecycle stage matching `RHGame::GameLoop`: dispatch
    /// mission `PostInitialize` after the first host refresh and sound
    /// hourglass.  Replay drivers must run this after reconstructing
    /// frame zero as well.
    pub fn perform_post_initialize(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
    ) -> Option<SideEffects> {
        self.require_live_campaign("performing mission PostInitialize");
        self.inner.perform_post_initialize(display, assets)
    }

    /// Apply one player command.  Commands are the only host → sim
    /// channel that mutates serialised state outside `perform_hourglass`.
    pub fn apply_command(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.require_live_campaign("applying a player command");
        let sim = self.inner.control.simulation_context();
        self.inner.apply_command(&sim, display, input, assets, cmd);
    }

    /// Apply a batch of player commands, as used by the replay driver
    /// and the rollback checker.
    pub fn apply_commands(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmds: &[PlayerInput],
    ) {
        self.require_live_campaign("applying replay or network commands");
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_commands(&sim, display, input, assets, cmds);
    }

    /// Apply a batch of locally-sourced commands (live single-player
    /// host pipeline). See [`EngineInner::apply_local_commands`].
    pub fn apply_local_commands(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmds: &[PlayerCommand],
    ) {
        self.require_live_campaign("applying local commands");
        self.inner
            .apply_local_commands(display, input, assets, cmds);
    }

    /// Fire the `DIES IRAE` (`EZEKIEL_2517`) cheat if active: instakill
    /// the target when the host's alt-hover gesture lands on a live
    /// human.  Returns `true` when the cheat consumed the gesture (the
    /// host should NOT then set `host.selected_view_element`).
    ///
    /// This is a cheat shortcut — the instakill is a sim-state mutation
    /// that bypasses the normal command recording.  Rollback replay of
    /// frames spanning the cheat activation may desync for one
    /// window; acceptable since EZEKIEL is a dev-only toggle rarely
    /// triggered during normal play.
    pub fn try_ezekiel_instakill(&mut self, id: EntityId) -> bool {
        self.inner.try_ezekiel_instakill(id)
    }

    /// Host-side entry point for injecting a `SimpleMessage` onto the
    /// engine messenger.  Used by UI sites that forward messages (console
    /// hide, switch-task, …); the drain handler in
    /// `perform_hourglass_inner` is the sole consumer.
    pub fn send_simple_message(&mut self, msg: crate::messenger::SimpleMessage) {
        self.inner.send_simple_message(msg);
    }

    // ── Setup / lifecycle ──────────────────────────────────────────

    pub fn replace_campaign_from_console(&mut self, campaign: Campaign) {
        self.inner.replace_campaign(campaign);
    }

    /// Run a host-side mission-script extension against live script effects
    /// while the Engine-owned simulation RNG is installed.
    ///
    /// Spellforge startup is outside the normal engine tick but its native
    /// shims can still draw from `sim_rng`; using this boundary advances the
    /// one authoritative stream instead of panicking for lack of a scope or
    /// inventing a second RNG. The closure must not retain the host reference.
    pub fn with_mission_script_effects_and_rng<R>(
        &mut self,
        assets: &LevelAssets,
        f: impl FnOnce(
            &crate::sim_rng::SimulationContext,
            Option<(
                &mut crate::natives::ScriptEffects,
                &mut crate::natives::ScriptState,
                &mut crate::engine::ScriptDomains,
                &crate::natives::AttachedScriptBindings,
                &crate::natives::NativeSessionCapabilities<'_>,
            )>,
        ) -> R,
    ) -> R {
        self.inner.with_simulation_context(|inner, sim| {
            if inner.scripts.mission.is_none() {
                return f(sim, None);
            }
            inner
                .with_script_session(sim, assets, |script, script_domains, capabilities| {
                    f(
                        sim,
                        Some((
                            &mut script.script_effects,
                            &mut script.state,
                            script_domains,
                            &script.bindings,
                            capabilities,
                        )),
                    )
                })
                .expect("mission script disappeared while opening the Lua script session")
        })
    }

    /// Consume a finished mission engine and return its campaign allocation.
    pub fn into_campaign(self) -> Campaign {
        self.inner.into_campaign()
    }

    /// Consume a finished mission engine while preserving the complete next
    /// RNG state for campaign selection before the following engine exists.
    pub fn into_campaign_and_simulation(self) -> (Campaign, u64, SimConfig) {
        let rng_seed = self.inner.rng_seed();
        let sim_config = self.inner.control.sim_config;
        (self.inner.into_campaign(), rng_seed, sim_config)
    }

    fn require_live_campaign(&self, context: &str) {
        self.inner.mission_domain.required_campaign(context);
    }

    /// Run a console-cheat input and return the dispatch response
    /// directly.  Console cheats are dev escape-hatches outside the
    /// command pipeline (not replay-tracked), so the response is
    /// transient UI text — no rollback-hash concern with returning it.
    ///
    /// `selected_view_element` is the host's alt-hover UI selection —
    /// cheats that operate on "the NPC you're currently viewing" read
    /// (and sometimes clear) it.
    pub fn run_console_command(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        let sim = self.inner.control.simulation_context();
        self.inner
            .run_console_command(&sim, assets, dev, selected_view_element, input)
    }

    /// Run a console-cheat input with the dev cheat set forced on, even
    /// if the console is currently in `use_final` mode.  Intended for
    /// out-of-band cheat entry points (HTTP RPC, debug overlays) whose
    /// caller contract is "always reach the full dev command table".
    pub fn run_cheat_string(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        selected_view_element: &mut Option<EntityId>,
        input: &str,
    ) -> ConsoleResponse {
        self.inner
            .run_cheat_string(assets, dev, selected_view_element, input)
    }

    pub fn restore_rng_from_seed(&mut self, seed: u64) {
        self.inner.restore_rng_from_seed(seed);
    }

    /// Complete deterministic configuration currently owned by this Engine.
    pub fn sim_config(&self) -> SimConfig {
        self.inner.control.sim_config
    }

    /// Seed and configuration captured before this mission's frame-0 setup.
    pub fn mission_start_simulation(&self) -> (u64, SimConfig) {
        (
            self.inner.control.mission_start_rng_seed,
            self.inner.control.mission_start_sim_config,
        )
    }

    /// Invoke a script `NativeFn` from outside the VM (HTTP-RPC, debug
    /// tooling).  See [`EngineInner::call_external_native`] for the full
    /// contract — runs through the same script-session boundary as engine
    /// callbacks, so any
    /// queued side-effects (camera, dialog, sequences, sound, deferred
    /// game-logic) are drained as if a script had made the call.
    pub fn call_external_native(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
    ) -> Result<i32, String> {
        let sim = self.inner.control.simulation_context();
        self.inner
            .call_external_native(&sim, assets, native_name, args)
    }

    /// Like [`Self::call_external_native`], but with an explicit transient
    /// `ThisActor` receiver.
    pub fn call_external_native_with_this(
        &mut self,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
        this_actor: Option<i32>,
    ) -> Result<i32, String> {
        let sim = self.inner.control.simulation_context();
        self.inner
            .call_external_native_with_this(&sim, assets, native_name, args, this_actor)
    }

    /// Refresh render-only patch door highlight flags.
    ///
    /// `Patch::display_doors` is intentionally outside serialization and the
    /// rollback hash, so this is allowed from the cursor/render path.
    /// Sim-visible mouse effects must still travel through `PlayerCommand`.
    pub fn refresh_selected_patch_display_doors(&mut self, selected_patch_idx: Option<u32>) {
        self.inner
            .refresh_selected_patch_display_doors(selected_patch_idx);
    }

    pub fn doors(&self) -> &[crate::gate::Door] {
        &self.inner.script_domains.interactables.doors
    }

    pub fn patches(&self) -> &[crate::patch::Patch] {
        &self.inner.script_domains.interactables.patches
    }

    // ── Per-frame drains ────
    // Patch-effect bg blits now travel through `SideEffects`
    // (`apply_side_effects` moves them into `Host::pending_bg_blits`)
    // so the engine no longer owns the queue between tick and render.

    // `mission_script_script_effects_mut` is no longer exposed — the
    // host-side callers go through `refresh_selected_patch_display_doors`
    // / `queue_update_information_bars` / `PlayerCommand::*` instead.

    // `campaign_mut` is no longer exposed — cross-crate callers use
    // the narrow methods below, or read through `campaign()` and
    // dispatch mutations via `PlayerCommand`.  `Campaign` is part of
    // the rollback hash; any future mutator added here must run on a
    // mission-lifecycle boundary (campaign map, save/load, quit) where
    // the sim is paused.

    /// Commit a blazon purchase on the owned campaign.  Pure menu-time
    /// operation — runs on the mission-description screen while the sim
    /// is paused.  Returns `true` when the Sherwood consume-cascade
    /// closed the buy screen (blazon mission fully funded), matching
    /// `Campaign::buy_blazon`.  `None` when no campaign is installed.
    pub fn campaign_buy_blazon(
        &mut self,
        mission_index: usize,
        profiles: &crate::profiles::ProfileManager,
    ) -> Option<bool> {
        let sim = self.inner.control.simulation_context();
        Some(
            self.inner
                .mission_domain
                .campaign
                .buy_blazon(&sim, mission_index, profiles),
        )
    }

    /// Reset the campaign's `last_pseudo_mission_status` flag after the
    /// campaign-map host has displayed the pseudo-mission debriefing.
    /// Runs on a mission-lifecycle boundary (sim paused) — `Campaign` is
    /// part of the rollback hash.  No-op when no campaign is installed.
    pub fn campaign_reset_last_pseudo_mission_status(&mut self) {
        self.inner
            .mission_domain
            .campaign
            .reset_last_pseudo_mission_status();
    }

    /// Reset the campaign's `MissionLength` accumulator to 0 before
    /// the mission begins.
    pub fn campaign_reset_mission_length(&mut self) {
        self.inner
            .mission_domain
            .campaign
            .set_value(crate::campaign::CampaignValue::MissionLength, 0);
    }

    /// Queue the `UpdateInformationBars` script-host command so the
    /// next tick rebuilds the blazon / requirements widgets against
    /// the current campaign state.  Called inline from
    /// `DisplayCampaignMap` post-commit and from the options-menu
    /// resolution-change handler.
    pub fn queue_update_information_bars(&mut self) {
        self.inner.queue_update_information_bars();
    }

    /// Push the active player profile's `GraphicConfig` through the
    /// shadow polygon and every live element so a graphics-options
    /// change takes effect immediately.
    ///
    /// Today neither effect needs explicit propagation: the
    /// shadow-polygon renderer reads the framed-view-cone flag live
    /// from the active profile at draw time (no cached function
    /// pointer), and element shadow rendering is not currently
    /// per-element-cached either.  The method is provided as a
    /// callable surface so the `DisplayMenu` re-entry path has a
    /// single hook — when the framed-view-cone shadow path or
    /// per-element shadow caching is wired up, the implementation
    /// here is the single point that needs to fan out the new config.
    pub fn change_detail_level(&mut self) {
        // Read-only access today (the active profile already supplies
        // GraphicConfig wherever rendering needs it).  Logged at debug
        // so the call shows up in replay traces alongside the
        // resolution-change events that surround it.
        tracing::debug!(
            "Engine::change_detail_level — graphics config refreshed (no cached state to invalidate)"
        );
    }

    pub fn is_peasant_name_registered(&self, name: &str) -> bool {
        self.inner.is_peasant_name_registered(name)
    }

    // ── Test-only helpers (round-trip save/load tests) ────────────
    //
    // Gated behind the `test-helpers` Cargo feature so production
    // builds of the facade do not expose direct sim-state setters.
    // `robin_rs` enables the feature in its `[dev-dependencies]`
    // block so its round-trip tests compile.

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_flags(&mut self, quit_won: bool, quit_lost: bool, mission_won: bool) {
        self.inner
            .test_set_mission_flags(quit_won, quit_lost, mission_won);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_frame_counter(&mut self, frame: u32) {
        self.inner.test_set_frame_counter(frame);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_engine_scalars(
        &mut self,
        cheat_used_flags: u32,
        speed: f32,
        speed_int: u16,
        lock_engine: bool,
        freeze_all: bool,
        script_globals: Vec<i32>,
    ) {
        self.inner.test_set_engine_scalars(
            cheat_used_flags,
            speed,
            speed_int,
            lock_engine,
            freeze_all,
            script_globals,
        );
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_mission_stat(&mut self, stat: crate::mission_stat::MissionStat) {
        self.inner.test_set_mission_stat(stat);
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_set_camera_transition_inputs(
        &mut self,
        zoom_init_done: bool,
        mechanized_zoom: bool,
        displacement: crate::coordinates::MapVec,
        displacement_counter: u16,
        pending_zoom_mouse_screen: Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &mut self.inner.feedback.cutscene_camera;
        camera.zoom_init_done = zoom_init_done;
        camera.mechanized_zoom = mechanized_zoom;
        camera.displacement = displacement;
        camera.displacement_counter = displacement_counter;
        camera.pending_zoom_mouse_screen = pending_zoom_mouse_screen;
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_camera_transition_inputs(
        &self,
    ) -> (
        bool,
        bool,
        crate::coordinates::MapVec,
        u16,
        Option<crate::coordinates::ScreenPoint>,
    ) {
        let camera = &self.inner.feedback.cutscene_camera;
        (
            camera.zoom_init_done,
            camera.mechanized_zoom,
            camera.displacement,
            camera.displacement_counter,
            camera.pending_zoom_mouse_screen,
        )
    }

    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_assert_level_assets_attached(&self, assets: &LevelAssets) {
        assert!(std::sync::Arc::ptr_eq(
            &self.inner.world.fast_grid.level,
            &assets.level_grid
        ));
        self.inner.scripts.assert_native_attachments_ready();
    }

    // ── Save / restore ────────────────────────────────────────────

    /// Build a fully-restored engine from a decoded save snapshot and the
    /// matching mission's immutable [`LevelAssets`].
    ///
    /// Wholesale sim-state replacement — legitimate because loading a
    /// save or rewinding is a deliberate, user-initiated discontinuity
    /// that also resets the rollback checker.  The consuming signature
    /// makes the replacement explicit at every call site: the old
    /// engine is moved in, a new one comes out.
    ///
    /// Everything mutable comes from `saved`. Static grid, sprite, script
    /// program, and native-binding attachments come only from `assets`, then
    /// [`EngineInner::post_load_fixups`] removes save-load-only transient
    /// state so it cannot leak into the resumed session.
    ///
    /// Queues `UpdateInformationBars` on the script host so the next
    /// tick recomputes the HUD state to match the loaded mission.
    pub fn restore(
        &mut self,
        display: &mut super::HostDisplayState,
        saved: Engine,
        assets: &LevelAssets,
    ) {
        // Assertion-oriented wrapper for callers that have already handled
        // snapshot compatibility. UI/file boundaries use `try_restore`.
        self.try_restore(display, saved, assets)
            .unwrap_or_else(|error| panic!("cannot restore engine snapshot: {error}"));
    }

    /// Fallible form of [`Self::restore`] for snapshot/network boundaries.
    ///
    /// Validation happens before `self` is mutated. In particular, malformed
    /// parallel fast-grid arrays are rejected rather than replaced with
    /// all-active values that were never present in the snapshot.
    pub fn try_restore(
        &mut self,
        display: &mut super::HostDisplayState,
        saved: Engine,
        assets: &LevelAssets,
    ) -> Result<(), SnapshotRestoreError> {
        self.try_restore_with_post_fixup_observer(display, saved, assets, |_| {})
    }

    fn try_restore_with_post_fixup_observer(
        &mut self,
        display: &mut super::HostDisplayState,
        saved: Engine,
        assets: &LevelAssets,
        post_fixup_observer: impl FnOnce(&EngineInner),
    ) -> Result<(), SnapshotRestoreError> {
        let mut inner = Self::prepare_snapshot(saved, assets)?;

        // ── Engine-owned transient reset + HUD refresh ───────────
        inner.post_load_fixups(display);
        post_fixup_observer(&inner);
        inner.queue_update_information_bars();
        self.inner = inner;
        Ok(())
    }

    /// Adopt an exact decoded network snapshot.
    ///
    /// Unlike save restore, adoption performs no transient resets and queues no
    /// repair messages: every serialized simulation queue remains exactly as
    /// sent by the host. The live engine changes only after the candidate has
    /// passed all attachment and state validation.
    pub fn try_adopt_snapshot(
        &mut self,
        snapshot: Engine,
        assets: &LevelAssets,
    ) -> Result<(), SnapshotRestoreError> {
        let inner = Self::prepare_snapshot(snapshot, assets)?;
        self.inner = inner;
        Ok(())
    }

    fn prepare_snapshot(
        snapshot: Engine,
        assets: &LevelAssets,
    ) -> Result<EngineInner, SnapshotRestoreError> {
        Self::validate_snapshot_compatibility(&snapshot, assets)?;
        let mut inner = snapshot.inner;

        // Attachment preflight above covers every static lookup, so this phase
        // cannot partially fail. The candidate is still detached from `self`.
        inner.attach_preflighted_level_assets(assets);
        inner.orders.sequence_manager.rebuild_indices();
        Ok(inner)
    }

    fn validate_snapshot_compatibility(
        saved: &Engine,
        assets: &LevelAssets,
    ) -> Result<(), SnapshotRestoreError> {
        let level = &assets.level_grid;
        let lengths = [
            (
                SnapshotGridComponent::Lines,
                saved.inner.world.fast_grid.line_active.len(),
                level.lines.len(),
            ),
            (
                SnapshotGridComponent::Sectors,
                saved.inner.world.fast_grid.sector_active.len(),
                level.sectors.len(),
            ),
            (
                SnapshotGridComponent::Masks,
                saved.inner.world.fast_grid.mask_active.len(),
                level.masks.len(),
            ),
        ];

        // Original provenance: `original-code/RHfastfindgrid.cpp:8890-9115`
        // serializes runtime patch/door/sector state against the already
        // loaded grid and propagates failure. It never invents an all-active
        // replacement when the save and level topology disagree.
        for (component, snapshot_len, level_len) in lengths {
            if snapshot_len != level_len {
                return Err(SnapshotRestoreError::FastGridLengthMismatch {
                    component,
                    snapshot_len,
                    level_len,
                });
            }
        }
        saved
            .inner
            .world
            .preflight_level_assets(assets, saved.inner.script_domains.zones.scripts.len())
            .map_err(|detail| SnapshotRestoreError::WorldInvariantViolation { detail })?;
        saved
            .inner
            .scripts
            .preflight_level_assets(assets)
            .map_err(|detail| SnapshotRestoreError::AttachmentFailure { detail })?;
        saved
            .inner
            .orders
            .validate_invariants()
            .map_err(|detail| SnapshotRestoreError::OrderInvariantViolation { detail })?;
        Ok(())
    }
}

impl Deref for Engine {
    type Target = EngineInner;

    fn deref(&self) -> &EngineInner {
        &self.inner
    }
}

// `Default for Engine` is intentionally not implemented: the RAII
// contract says an `Engine` exists only when it's a fully-initialised
// mission engine, and the required mission data can't be conjured
// from defaults.  Tests that want a blank engine should construct
// `EngineInner` directly (it stays `pub(crate)` for that use), or
// fabricate a test-fixture level and go through `Engine::new`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_snapshot_omits_only_nonserializable_original_rng_replay() {
        let mut inner = EngineInner::new();
        inner.control.rng = SimulationRng::with_original_replay(vec![11, 22]);
        let engine = Engine { inner };

        assert!(serde_json::to_value(&engine).is_err());
        let diagnostic = engine.diagnostic_snapshot_without_original_rng_replay();

        assert_eq!(engine.original_rng_replay_cursor(), Some(0));
        assert_eq!(diagnostic.original_rng_replay_cursor(), None);
        serde_json::to_value(&diagnostic).expect("diagnostic engine must serialize");
    }

    #[test]
    fn campaign_selection_transfers_one_rng_sequence_to_mission_construction() {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles.missions.push(crate::profiles::MissionProfile {
            id: 0,
            location: crate::profiles::MissionLocation::Sherwood,
            life_time: 100,
            max_ransom: 200_000,
            max_gang_size: u16::MAX,
            ..Default::default()
        });
        for id in 1..=2 {
            profiles.missions.push(crate::profiles::MissionProfile {
                id,
                mission_type: crate::profiles::MissionType::Rescue,
                location: crate::profiles::MissionLocation::York,
                life_time: 100,
                access_probability: 50,
                max_ransom: 200_000,
                max_gang_size: u16::MAX,
                ..Default::default()
            });
        }

        let mut campaign = Campaign::default();
        for profile_idx in 0..3 {
            campaign.missions.push(crate::mission::Mission {
                profile_idx: Some(profile_idx),
                ..Default::default()
            });
        }
        campaign.accessible_mission_indices = vec![1, 2];

        let seed = 0xCA11_AB1E;
        let config = SimConfig::default();
        let reference_context =
            crate::sim_rng::SimulationContext::with_seed_and_config(seed, config);
        let mut reference_campaign = campaign.clone();
        let expected_mission =
            reference_campaign.determine_next_mission(&reference_context, &profiles);
        let expected_next_seed = reference_context.seed();
        let expected_next_draw = crate::sim_rng::u32(
            &reference_context,
            crate::sim_rng::RngSite::TitbitUpdate,
            ..,
        );

        let (_campaign, mission, next_seed, next_config) =
            Engine::select_next_mission(campaign, &profiles, seed, config);
        let mission_context =
            crate::sim_rng::SimulationContext::with_seed_and_config(next_seed, config);
        let actual_next_draw =
            crate::sim_rng::u32(&mission_context, crate::sim_rng::RngSite::TitbitUpdate, ..);

        assert_eq!(mission, expected_mission);
        assert_eq!(next_seed, expected_next_seed);
        assert_eq!(next_config, config);
        assert_eq!(actual_next_draw, expected_next_draw);
    }

    fn scripted_snapshot_fixture() -> (
        Engine,
        LevelAssets,
        std::sync::Arc<crate::script_manager::ScriptProgram>,
        crate::sequence::SequenceId,
    ) {
        let scb = crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "snapshot_attachment_test.scs".to_owned(),
                class_name: "StartUp".to_owned(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        };
        let program = std::sync::Arc::new(crate::script_manager::ScriptProgram::from_scb(scb));
        let script_name = "snapshot_attachment_test".to_owned();
        let script =
            crate::engine::MissionScript::from_program(script_name.clone(), program.clone())
                .expect("minimal StartUp script");

        let mut assets = LevelAssets::new();
        assets.scripts.mission_name = Some(script_name.clone());
        std::sync::Arc::make_mut(&mut assets.scripts.mission_programs)
            .insert(script_name, program.clone());

        let mut inner = EngineInner::new();
        inner.scripts.install_mission(script);
        inner.scripts.attach_native_capabilities(&assets);
        inner
            .scripts
            .mission
            .as_mut()
            .expect("fixture mission script")
            .script_effects
            .emit_engine(crate::natives::EngineCommand::UpdateInformationBars);
        {
            let effects = &mut inner
                .scripts
                .mission
                .as_mut()
                .expect("fixture mission script")
                .script_effects;
            effects.emit_sound(crate::natives::SoundCommand::SuspendAll);
            effects.emit_engine(crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 });
            effects.emit_barrier(crate::natives::DeferredCommand::FreezeAll { freeze: true });
        }

        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Generic,
            None,
        ));
        let sequence_id = inner.orders.sequence_manager.launch_sequence(sequence);

        (Engine { inner }, assets, program, sequence_id)
    }

    fn decoded_engine(engine: &Engine) -> Engine {
        serde_json::from_str(&serde_json::to_string(engine).expect("serialize engine snapshot"))
            .expect("decode engine snapshot")
    }

    #[test]
    fn failed_construction_returns_the_same_campaign_allocation() {
        let mut profiles = crate::profiles::ProfileManager::default();
        profiles
            .missions
            .push(crate::profiles::MissionProfile::default());
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            filename: "missing-construction-test-sprite".to_owned(),
            profile_name: "missing-construction-test-profile".to_owned(),
            ..crate::profiles::SoldierProfile::default()
        });

        let mut campaign = crate::campaign::Campaign::default();
        campaign.missions.push(crate::mission::Mission {
            profile_idx: Some(0),
            ..crate::mission::Mission::default()
        });
        campaign.current_mission_idx = Some(0);
        campaign.missions.reserve_exact(257);
        assert!(!campaign.missions.is_empty());
        let missions = campaign.missions.as_ptr();
        let mission_capacity = campaign.missions.capacity();

        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(profiles);
        let mut loaded = crate::level_data::LoadedLevel::empty_for_test();
        loaded.mission.soldiers.push(crate::level_data::RawSoldier {
            position_x: 0,
            position_y: 0,
            direction: 0,
            action: 0,
            obstacle_index: 0,
            sector: 0,
            layer: 0,
            material: 0,
            profile_number: 0,
            tower_guard: false,
            company_number: 0,
            drunk_level: 0,
            money: 0,
            subordinate_ids: Vec::new(),
            path_id: 0,
            alert_path_id: 0,
            script_class: None,
        });

        let result = Engine::new_preserving_campaign(EngineArgs {
            campaign,
            level: LevelLoadArgs {
                assets: &mut assets,
                level_directory: "",
                progress: &mut |_| {},
                loaded,
                bg_pixel_dims: (0.0, 0.0),
            },
            ground_mark_sprite: None,
            titbit_row_frame_counts: Vec::new(),
            rng_seed: 0,
            original_rng_replay: None,
            sim_config: SimConfig::default(),
        });

        let (error, returned) = match result {
            Ok(_) => panic!("missing sprite must fail construction"),
            Err(failure) => failure,
        };
        assert!(matches!(error, EngineError::ProfileSpriteLoadFailed { .. }));
        assert_eq!(returned.missions.as_ptr(), missions);
        assert_eq!(returned.missions.capacity(), mission_capacity);
        assert_eq!(returned.current_mission_idx, Some(0));
    }

    /// Serialized camera state belongs to the snapshot; the previous live
    /// engine is not an attachment source. Only immutable `LevelAssets` are
    /// admitted by the preparation path.
    #[test]
    fn restore_uses_snapshot_camera_state_not_previous_engine() {
        let mut source_inner = EngineInner::new();

        source_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);

        let source = Engine {
            inner: source_inner,
        };

        let json = serde_json::to_string(&source).expect("serialize");
        let decoded: Engine = serde_json::from_str(&json).expect("deserialize");

        let mut restored = source;
        restored.inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(99.0, 88.0);
        let mut display = crate::engine::HostDisplayState::default();
        restored.restore(&mut display, decoded, &LevelAssets::new());

        assert_eq!(
            restored.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0)
        );
    }

    #[test]
    fn try_restore_rejects_mismatched_runtime_lengths_without_mutating_live_engine() {
        let mut live_inner = EngineInner::new();
        live_inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(1234.0, 5678.0);
        let mut live = Engine { inner: live_inner };

        let mut malformed_inner = EngineInner::new();
        malformed_inner.world.fast_grid.line_active.push(true);
        let malformed = Engine {
            inner: malformed_inner,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = live
            .try_restore(&mut display, malformed, &LevelAssets::new())
            .unwrap_err();
        assert_eq!(
            error,
            SnapshotRestoreError::FastGridLengthMismatch {
                component: SnapshotGridComponent::Lines,
                snapshot_len: 1,
                level_len: 0,
            }
        );
        assert_eq!(
            live.inner.feedback.cutscene_camera.level_size,
            crate::coordinates::MapSize::new(1234.0, 5678.0),
            "validation must happen before replacing the live engine"
        );
    }

    #[test]
    fn try_restore_rejects_world_parallel_mismatch_before_mutating_live_engine() {
        let mut live = Engine {
            inner: EngineInner::new(),
        };
        let mut malformed_inner = EngineInner::new();
        malformed_inner
            .script_domains
            .zones
            .scripts
            .push(crate::sector::ScriptSectorData::new());
        let malformed = Engine {
            inner: malformed_inner,
        };

        let mut display = crate::engine::HostDisplayState::default();
        let error = live
            .try_restore(&mut display, malformed, &LevelAssets::new())
            .unwrap_err();
        assert_eq!(
            error,
            SnapshotRestoreError::WorldInvariantViolation {
                detail: "script-zone runtime length 1 does not match level zone-index length 0"
                    .to_owned(),
            }
        );
        assert!(live.inner.script_domains.zones.scripts.is_empty());
    }

    #[test]
    fn network_adoption_is_fully_attached_and_preserves_hash_and_script_queue() {
        let (source, assets, program, sequence_id) = scripted_snapshot_fixture();
        let source_hash = crate::replay::state_hash(&source);
        let snapshot = decoded_engine(&source);
        let mut live = Engine {
            inner: EngineInner::new(),
        };

        live.try_adopt_snapshot(snapshot, &assets)
            .expect("adopt compatible snapshot");

        assert_eq!(crate::replay::state_hash(&live), source_hash);
        live.inner.scripts.assert_native_attachments_ready();
        let script = live.inner.scripts.mission.as_ref().expect("adopted script");
        assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
        assert!(std::sync::Arc::ptr_eq(
            &script.bindings.profile_manager,
            &assets.profile_manager
        ));
        assert!(matches!(
            script.script_effects.ordered.as_slices(),
            (
                [
                    crate::natives::ScriptEffect::Presentation(
                        crate::natives::EngineCommand::UpdateInformationBars
                    ),
                    crate::natives::ScriptEffect::ExternalSound(
                        crate::natives::SoundCommand::SuspendAll
                    ),
                    crate::natives::ScriptEffect::Simulation(
                        crate::natives::SimulationEffect::Engine(
                            crate::natives::EngineCommand::ChooseVictoryDefeatText { id: 17 }
                        )
                    ),
                    crate::natives::ScriptEffect::Simulation(
                        crate::natives::SimulationEffect::Deferred(
                            crate::natives::DeferredCommand::FreezeAll { freeze: true }
                        )
                    )
                ],
                []
            )
        ));
        assert_eq!(
            live.inner
                .orders
                .sequence_manager
                .get_sequence(sequence_id)
                .map(|sequence| sequence.id),
            Some(sequence_id),
            "serialized sequences must be addressable after lookup indices rebuild"
        );
    }

    #[test]
    fn save_restore_attaches_before_fixups_and_appends_save_only_hud_repair() {
        let (mut source, assets, program, _) = scripted_snapshot_fixture();
        source.inner.feedback.cutscene_camera.level_size =
            crate::coordinates::MapSize::new(4096.0, 4096.0);
        source
            .inner
            .feedback
            .cutscene_camera
            .display
            .background_transform
            .zoom_to_up = true;
        source.inner.feedback.cutscene_camera.zoom_init_done = true;
        let queued_engine_commands = source
            .inner
            .scripts
            .mission
            .as_ref()
            .expect("fixture script")
            .script_effects
            .engine_commands()
            .len();
        let snapshot = decoded_engine(&source);
        let mut live = Engine {
            inner: EngineInner::new(),
        };
        let mut display = super::super::HostDisplayState::default();

        let observed_fixups_before_hud_repair = std::cell::Cell::new(false);
        live.try_restore_with_post_fixup_observer(&mut display, snapshot, &assets, |inner| {
            observed_fixups_before_hud_repair.set(true);
            assert_eq!(
                inner.orders.messenger.count(),
                3,
                "zoom-end, stature, and select-action must already be queued"
            );
            assert_eq!(
                inner
                    .scripts
                    .mission
                    .as_ref()
                    .expect("restored script during fixup observation")
                    .script_effects
                    .engine_commands()
                    .len(),
                queued_engine_commands,
                "save-only HUD repair must not be queued until engine fixups finish"
            );
        })
        .expect("restore compatible save snapshot");
        assert!(observed_fixups_before_hud_repair.get());

        live.inner.scripts.assert_native_attachments_ready();
        let script = live
            .inner
            .scripts
            .mission
            .as_ref()
            .expect("restored script");
        assert!(std::sync::Arc::ptr_eq(&script.manager.program, &program));
        assert_eq!(
            script.script_effects.engine_commands().len(),
            queued_engine_commands + 1,
            "saved queue must survive and save-load must append one HUD repair"
        );
        let messages = live.inner.orders.messenger.drain();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0].msg_type,
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::ZoomUpEnd)
        );
        assert_eq!(
            messages[1].msg_type,
            crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature)
        );
        assert!(matches!(
            messages[2].msg_type,
            crate::messenger::MessageType::Pc(crate::messenger::PcMessage::SelectAction, _)
        ));
        assert_eq!(display.display_op, crate::engine::DisplayOpCode::Redraw);
    }

    #[test]
    fn failed_attachment_preflight_does_not_mutate_live_engine() {
        let (source, mut assets, _, _) = scripted_snapshot_fixture();
        let snapshot = decoded_engine(&source);
        assets.scripts.mission_programs = std::sync::Arc::new(std::collections::BTreeMap::new());

        let mut live_inner = EngineInner::new();
        live_inner.control.frame_counter = 77;
        let mut live = Engine { inner: live_inner };
        let before_hash = crate::replay::state_hash(&live);

        let error = live.try_adopt_snapshot(snapshot, &assets).unwrap_err();
        assert!(matches!(
            error,
            SnapshotRestoreError::AttachmentFailure { ref detail }
                if detail.contains("missing mission script program 'snapshot_attachment_test'")
        ));
        assert_eq!(crate::replay::state_hash(&live), before_hash);
        assert_eq!(live.frame_counter(), 77);
    }

    #[test]
    fn adoption_rejects_wrong_loaded_mission_identity() {
        let (source, mut assets, _, _) = scripted_snapshot_fixture();
        let snapshot = decoded_engine(&source);
        assets.scripts.mission_name = Some("different_mission".to_owned());
        let mut live = Engine {
            inner: EngineInner::new(),
        };

        let error = live.try_adopt_snapshot(snapshot, &assets).unwrap_err();
        assert!(matches!(
            error,
            SnapshotRestoreError::AttachmentFailure { ref detail }
                if detail.contains("does not match loaded mission script 'different_mission'")
        ));
        assert!(live.inner.scripts.mission.is_none());
    }

    #[test]
    fn adoption_rejects_mobile_count_from_level_assets_atomically() {
        let mut assets = LevelAssets::new();
        assets.entities.mobile_element_count = 1;
        let snapshot = Engine {
            inner: EngineInner::new(),
        };
        let mut live_inner = EngineInner::new();
        live_inner.control.frame_counter = 91;
        let mut live = Engine { inner: live_inner };
        let before_hash = crate::replay::state_hash(&live);

        let error = live.try_adopt_snapshot(snapshot, &assets).unwrap_err();
        assert_eq!(
            error,
            SnapshotRestoreError::WorldInvariantViolation {
                detail: "snapshot mobile-element count 0 does not match loaded level count 1"
                    .to_owned()
            }
        );
        assert_eq!(crate::replay::state_hash(&live), before_hash);
        assert_eq!(live.frame_counter(), 91);
    }
}
