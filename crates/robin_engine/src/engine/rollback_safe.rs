//! Cross-crate `Engine` facade that makes the
//! "mutations-only-inside-the-tick" invariant mechanical.
//!
//! Downstream crates only ever see the [`Engine`] wrapper defined here.
//! It gives read-only access to the underlying [`EngineInner`] via
//! `Deref`; there is no `DerefMut` and no accessor returning
//! `&mut EngineInner`, so the only way to mutate simulation state from
//! outside `robin_engine` is through an explicit method on this type.
//!
//! [`Engine::advance_frame`] is the canonical runtime transaction. Remaining
//! exposed mutators are compatibility/setup seams and fall into one of these
//! categories:
//!
//! * legacy command/hourglass calls retained for engine fixtures and migration,
//! * a one-shot setup / level-load / lifecycle hook, or
//! * a drain of a side-effect queue filled during the tick and consumed
//!   host-side.
//!
//! Anything that doesn't fit one of those buckets should be pushed into
//! the sim via `SimulationFrameInput`, not added here.

use std::ops::Deref;

use super::SimConfig;
use super::{
    ConsoleResponse, DevState, EngineError, EngineInner, ExternalAction, ExternalActionResult,
    ExternalFacts, FrameAdvanceError, FrameConsoleResponse, InputState, LevelAssets,
    LevelLoadStaging, SideEffects, SimEvents, SimulationFrameInput, SimulationFrameOutput,
    SimulationRng, SoundBoundaryPolicy,
};
#[cfg(test)]
use super::{DirectorCompletion, SoundBoundary};
use crate::campaign::Campaign;
use crate::element::EntityId;
use crate::minimap::HitMask;
#[cfg(test)]
use crate::player_command::PlayerCommand;
use crate::player_command::PlayerInput;

/// Canonical gameplay-authoritative engine scalars emitted by schema-13
/// Original parity traces. Presentation camera/surface/backend state is
/// deliberately absent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ParityEngineState {
    pub cheat_used_flags: u32,
    pub next_creation_order: u32,
    pub chorus_timer: u16,
    pub force_check: bool,
    pub men_to_blazon_conversion: bool,
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
    /// Restore an Original schema-16 session-boundary transient before the
    /// first replay frame. The v48 save payload does not carry this field.
    #[doc(hidden)]
    pub fn restore_parity_npc_maximal_visibility(&mut self, id: EntityId, value: u16) {
        self.inner.restore_parity_npc_maximal_visibility(id, value);
    }

    #[doc(hidden)]
    pub fn restore_parity_npc_dormant_macro_cursor(
        &mut self,
        id: EntityId,
        path_id: crate::ai::PathId,
        waypoint_index: u8,
        offset: usize,
        assets: &LevelAssets,
    ) -> bool {
        self.inner.restore_parity_npc_dormant_macro_cursor(
            id,
            path_id,
            waypoint_index,
            offset,
            assets,
        )
    }

    /// Complete serialized position and sprite frontier for one entity.
    #[doc(hidden)]
    pub fn parity_entity_runtime_state(
        &self,
        id: EntityId,
        assets: &LevelAssets,
    ) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity_ref = |id: EntityId| {
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
        let entity = self.inner.world.entities.get(id).unwrap_or_else(|| {
            panic!("parity runtime projection references missing entity {id:?}")
        });
        let sprite = &entity.element_data().sprite;
        let position = sprite.position_iface.v48_serialized_state();
        let float = |value: f32| json!({ "bits": value.to_bits(), "value": value });
        let point2 = |x: f32, y: f32| json!({ "x": float(x), "y": float(y) });
        let point3 =
            |x: f32, y: f32, z: f32| json!({ "x": float(x), "y": float(y), "z": float(z) });
        let jump_line = |index: Option<u32>| -> Value {
            let Some(index) = index else {
                return Value::Null;
            };
            let line = self
                .inner
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::try_from(index).expect("parity enemy jump-line index exceeds usize"))
                .unwrap_or_else(|| panic!("parity enemy references missing jump line {index}"));
            json!({
                "a": point2(line.point_a.x, line.point_a.y),
                "b": point2(line.point_b.x, line.point_b.y),
            })
        };
        let bbox = |bbox: crate::coordinates::MapBBox| match bbox.0 {
            Some(rect) => json!({
                "min": point2(rect.min().x, rect.min().y),
                "max": point2(rect.max().x, rect.max().y),
            }),
            None => Value::Null,
        };
        let sector = |handle: Option<crate::position_interface::SectorHandle>| {
            handle.map_or(Value::Null, |handle| {
                let sector = self
                    .inner
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(handle.get() as usize)
                    .unwrap_or_else(|| {
                        panic!("parity position references missing sector {handle}")
                    });
                json!(sector.sector_number.get())
            })
        };
        let target = position.target_element.map_or(Value::Null, entity_ref);
        let door = if position.door.is_null() {
            Value::Null
        } else {
            let index = usize::try_from(position.door.0)
                .unwrap_or_else(|_| panic!("parity position door index exceeds usize"));
            let door = self
                .inner
                .script_domains
                .interactables
                .doors
                .get(index)
                .unwrap_or_else(|| panic!("parity position references missing door {index}"));
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
                "point_out": point2(door.point_out.x, door.point_out.y),
                "point_in": point2(door.point_in.x, door.point_in.y),
            })
        };
        let obstacle = position.obstacle.map_or(Value::Null, |handle| {
            let handle = usize::from(handle);
            let obstacle = assets
                .static_sight_obstacles
                .get(handle)
                .unwrap_or_else(|| {
                    panic!("parity position references missing static obstacle {handle}")
                });
            if position.layer.get() == u16::MAX {
                json!({ "kind": "sight", "index": obstacle.id })
            } else {
                let index = assets.static_sight_obstacles[..handle]
                    .iter()
                    .filter(|candidate| candidate.is_projection_area())
                    .count();
                if !obstacle.is_projection_area() {
                    panic!(
                        "parity position on layer {} references non-projection obstacle {handle}",
                        position.layer.get()
                    );
                }
                json!({ "kind": "projection", "index": index })
            }
        });
        if sprite.anims_to_be_replaced.len() != sprite.replacing_anims.len() {
            panic!(
                "sprite replacement list length {} differs from replacement value length {} for {id:?}",
                sprite.anims_to_be_replaced.len(),
                sprite.replacing_anims.len()
            );
        }
        let replacements = sprite
            .anims_to_be_replaced
            .iter()
            .zip(&sprite.replacing_anims)
            .map(|(&from, &to)| json!({ "from": from as u32, "to": to as u32 }))
            .collect::<Vec<_>>();

        // Keep these in bounded chunks: a single `json!` object containing
        // the entire serialized position frontier exceeds the macro's normal
        // recursion limit.
        let mut position_state = json!({
                "computed_position": position.computed_position.bits(),
                "computed_increment": position.computed_increment.bits(),
                "material": position.material,
                "posture": position.posture as u32,
                "old_posture": position.old_posture as u32,
                "direction": i16::from(position.direction),
                "direction_goal": i16::from(position.direction_goal),
                "slow_turn_count": position.slow_turn_count,
                "direction_count": position.direction_count,
                "layer": position.layer.get(), "layer_goal": position.layer_goal.get(),
                "tolerance": float(position.tolerance),
                "directional_tolerance": position.directional_tolerance,
                "accumulate_movement_map": position.accumulate_movement_map,
                "anti_collision_on": position.anti_collision_on,
                "goal_next_valid": position.goal_next_valid,
                "deviated": position.deviated,
                "door_direction": position.door_direction,
                "reversed_movement": position.reversed_movement,
                "blocked_count": position.blocked_count,
                "radius": float(position.radius),
                "emergency_lying_box": position.use_emergency_lying_box,
                "sector": sector(position.sector), "sector_goal": sector(position.sector_goal),
                "door": door, "obstacle": obstacle, "target": target,
        });
        position_state
            .as_object_mut()
            .expect("parity position chunk must be an object")
            .extend(
                json!({
                "world": point3(position.position.x, position.position.y, position.position.z),
                "map": point2(position.map.x, position.map.y),
                "sprite": point2(position.sprite.x, position.sprite.y),
                "old_world": point3(position.old_position.x, position.old_position.y, position.old_position.z),
                "old_map": point2(position.old_map.x, position.old_map.y),
                "old_sprite": point2(position.old_sprite.x, position.old_sprite.y),
                "goal_map": point2(position.goal_map.x, position.goal_map.y),
                "goal_next_map": point2(position.goal_next_map.x, position.goal_next_map.y),
                "goal_world": point3(position.goal.x, position.goal.y, position.goal.z),
                "increment": point3(position.increment.x, position.increment.y, position.increment.z),
                "increment_map": point2(position.increment_map.x, position.increment_map.y),
                "accumulated_movement_map": point2(position.accumulated_movement_map.x, position.accumulated_movement_map.y),
                "forecasted_movement": point3(position.forecasted_movement.x, position.forecasted_movement.y, position.forecasted_movement.z),
                "move_box": bbox(position.move_box_map), "blocked_box": bbox(position.blocked_box),
                })
                .as_object()
                .expect("parity position geometry chunk must be an object")
                .clone(),
            );
        let sprite_state = json!({
                "row": sprite.current_row, "frame": sprite.current_frame,
                "frame_count": sprite.frame_count,
                "flight_countdown": sprite.flight_frame_countdown,
                "width": sprite.current_width, "height": sprite.current_height,
                "last_action": sprite.last_action as u32,
                "last_processed_order_id": sprite.last_processed_order_id,
                "masked": sprite.masked, "alternate_profile": sprite.use_alternate_profile,
                "action_done_frame": sprite.action_done_frame,
                "action_done_counter": sprite.action_done_counter,
                "last_sound_id": sprite.last_sound_id,
                "behind_display_order_reference": sprite.behind_display_order_ref,
                "display_order_reference": sprite.display_order_ref.map_or(Value::Null, entity_ref),
                "replacements": replacements,
        });

        let projectile_state = |projectile: &crate::element::ProjectileData| {
            let trajectory = projectile
                .trajectory
                .iter()
                .enumerate()
                .map(|(index, point)| {
                    let runtime = projectile.trajectory_runtime.get(index);
                    json!({
                        "position": point3(point.position.x, point.position.y, point.position.z),
                        "time": point.time,
                        // `null` is an explicit incomplete runtime mirror, not a fabricated
                        // non-bounce/material value. Fresh trajectory construction must
                        // populate these fields before v29 can pass dynamic-projectile traces.
                        "bounce": runtime.map(|runtime| runtime.bounce),
                        "material": runtime.map(|runtime| runtime.material),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "flying": projectile.flying,
                "dive": projectile.dive,
                "magic_bullet": projectile.magic_bullet,
                "frame_count": projectile.frame_count,
                "trajectory_origin": {
                    "map": point2(projectile.start_of_trajectory_x, projectile.start_of_trajectory_y),
                    "sector": projectile.trajectory_origin_sector,
                    "layer": projectile.trajectory_origin_layer,
                },
                "flight_direction": projectile.flight_direction,
                "start": point3(projectile.start.x, projectile.start.y, projectile.start.z),
                "end": point3(projectile.end.x, projectile.end.y, projectile.end.z),
                "shooter": projectile.shooter.map_or(Value::Null, entity_ref),
                "trajectory": trajectory,
            })
        };
        let resolve_ai_handle = |handle: u32| -> Value {
            if handle == 0 {
                return Value::Null;
            }
            let resolved = self
                .inner
                .world
                .entities
                .occupied()
                .find_map(|(candidate, _)| (candidate.index() == handle).then_some(candidate))
                .unwrap_or_else(|| panic!("parity local AI references missing handle {handle}"));
            entity_ref(resolved)
        };
        let ai_position = |position: crate::ai::Position| {
            json!({
                "map": point2(position.x, position.y),
                "sector": sector(position.sector),
                "layer": position.level,
            })
        };
        let seek_point = |point: &crate::ai::SeekPoint| {
            json!({
                "position": ai_position(point.position),
                "frame_when_full_interest": point.frame_when_full_interest,
                "directions": &point.directions,
                "last_calculated_interest": point.last_calculated_interest,
                "locked": point.locked,
            })
        };
        let known_strike_command = |strike: Option<crate::weapons::SwordStrike>| -> i32 {
            use crate::{element::Command, weapons::SwordStrike};
            match strike {
                None => Command::Null as i32,
                Some(SwordStrike::A) => Command::SwordstrikeThrustA as i32,
                Some(SwordStrike::B) => Command::SwordstrikeThrustB as i32,
                Some(SwordStrike::C) => Command::SwordstrikeThrustC as i32,
                Some(SwordStrike::D) => Command::SwordstrikeThrustD as i32,
                Some(SwordStrike::E) => Command::SwordstrikeThrustE as i32,
                Some(SwordStrike::F) => Command::SwordstrikeThrustF as i32,
                Some(SwordStrike::G) => Command::SwordstrikeThrustG as i32,
                Some(SwordStrike::H) => Command::SwordstrikeThrustH as i32,
                Some(SwordStrike::I) => Command::SwordstrikeThrustI as i32,
                Some(other) => {
                    panic!("parity enemy known-strike slot contains invalid strike {other:?}")
                }
            }
        };
        let stimulus_state = |stimulus: &crate::ai::Stimulus| -> Value {
            use crate::ai::{StimulusInfo, StimulusType};
            assert_ne!(
                stimulus.stimulus_type,
                StimulusType::ForceBattleDecision,
                "parity local-AI stimulus contains Rust-only non-serializable type",
            );
            let (info_type, info) = match stimulus.info {
                StimulusInfo::None => (0, json!({ "kind": "none" })),
                StimulusInfo::Noise(noise) => (
                    1,
                    json!({
                        "kind": "noise", "origin": ai_position(noise.origin),
                        "noise_type": noise.noise_type as u32,
                        "volume": noise.volume, "elevation": noise.elevation,
                    }),
                ),
                StimulusInfo::Position(position) => (
                    2,
                    json!({
                        "kind": "position", "position": ai_position(position),
                    }),
                ),
                StimulusInfo::Human(entity) => (
                    3,
                    json!({
                        "kind": "human", "entity": resolve_ai_handle(entity),
                    }),
                ),
                StimulusInfo::Hint(hint) => (
                    4,
                    json!({
                        "kind": "hint", "position": ai_position(hint.seek_point),
                        "teller": resolve_ai_handle(hint.who_tells_me), "seek_flags": hint.seek_flags,
                    }),
                ),
                StimulusInfo::Object(entity) => (
                    5,
                    json!({
                        "kind": "object", "entity": resolve_ai_handle(entity),
                    }),
                ),
                StimulusInfo::Stolen(stolen) => (
                    6,
                    json!({
                        "kind": "stolen", "object": resolve_ai_handle(stolen.object),
                        "thief": resolve_ai_handle(stolen.thief),
                    }),
                ),
                StimulusInfo::Combat(combat) => (
                    7,
                    json!({
                        "kind": "combat", "actor": resolve_ai_handle(combat.actor_npc),
                        "enemy_position": ai_position(combat.enemy_position),
                    }),
                ),
                StimulusInfo::DoorCombat(combat) => (
                    8,
                    json!({
                        "kind": "door_combat", "delay": combat.delay, "direction": combat.direction,
                        "goal": ai_position(combat.goal),
                        "adversary": resolve_ai_handle(combat.adversary),
                    }),
                ),
                StimulusInfo::Index(value) => (9, json!({ "kind": "index", "value": value })),
                StimulusInfo::LegacyInvalidType(raw) => {
                    panic!("parity local-AI stimulus retains active invalid type word {raw}")
                }
            };
            json!({
                "stimulus_type": stimulus.stimulus_type as u32,
                "info_type": info_type,
                "owner": resolve_ai_handle(stimulus.owner),
                "to_whole_patrol": stimulus.to_whole_patrol,
                "info": info,
            })
        };
        let patrol_stimulus = |stimulus: Option<&crate::ai::Stimulus>| -> Value {
            use crate::ai::{StimulusInfo, StimulusType};
            let Some(stimulus) = stimulus else {
                return Value::Null;
            };
            let is_default = stimulus.stimulus_type == StimulusType::NoEvent
                && matches!(
                    stimulus.info,
                    StimulusInfo::None | StimulusInfo::LegacyInvalidType(_)
                )
                && stimulus.owner == 0
                && !stimulus.to_whole_patrol;
            if is_default {
                Value::Null
            } else {
                stimulus_state(stimulus)
            }
        };
        let npc_ai = entity.npc_data().and_then(|npc| {
			let ai = npc.ai_brain.base()?;
			let ai_door = |index: Option<u32>| -> Value {
				let Some(index) = index else { return Value::Null };
				let door = self
					.inner
					.script_domains
					.interactables
					.doors
					.get(usize::try_from(index).expect("parity AI door index exceeds usize"))
					.unwrap_or_else(|| panic!("parity AI references missing door {index}"));
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
					"point_out": point2(door.point_out.x, door.point_out.y),
					"point_in": point2(door.point_in.x, door.point_in.y),
				})
			};
			let handles = |values: &[u32]| {
				values
					.iter()
					.copied()
					.map(resolve_ai_handle)
					.collect::<Vec<_>>()
			};
			let patrol_path_status = if let Some(path) = &ai.patrol_path {
				json!({
					"current_waypoint_index": path.current_waypoint_index,
					"last_waypoint_index": path.last_waypoint_index,
					"forward": path.forward,
					"hiking_path_index": path.hiking_path_index.get(),
					"history": path.history.iter().map(|entry| json!({
						"position": ai_position(entry.position), "direction": entry.direction,
						"distance": entry.distance,
					})).collect::<Vec<_>>(),
				})
			} else {
				let path = &ai.detached_patrol_path_status;
				json!({
					"current_waypoint_index": path.current_waypoint_index,
					"last_waypoint_index": path.last_waypoint_index,
					"forward": path.forward,
					"hiking_path_index": path.hiking_path_index.map_or(Value::Null, |id| json!(id.get())),
					"history": path.history.iter().map(|entry| json!({
						"position": ai_position(entry.position), "direction": entry.direction,
						"distance": entry.distance,
					})).collect::<Vec<_>>(),
				})
			};
			let mut state = json!({
				"last_goto": {
					"destination": ai_position(ai.last_goto_destination),
					"flags": ai.last_goto_flags.bits(), "stuck_counter": ai.stuck_counter,
				},
				"forbidden_remarks": ai.forbidden_remark_ids,
				"current_remark_flags": ai.current_remark_flags,
				"owner": ai.owner_entity_id.map_or(Value::Null, entity_ref),
				"state": ai.current_state as u32, "old_state": ai.old_state,
				"substate": ai.current_substate as u32,
				"music_alert": ai.current_music_alert_status as u32,
				"timer_launch_substate": ai.substate_at_last_timer_launch as u32,
				"attitude": ai.attitude as u32, "blood_alcohol": ai.blood_alcohol,
				"initial_action": ai.initial_action, "number_of_looks": ai.number_of_looks,
				"can_move": ai.can_move,
				"path_control": {
					"stop_before_end": ai.stop_before_end_of_path,
					"use_max_norm": ai.use_max_norm_to_stop_before_end_of_path,
					"stop_distance": ai.stop_before_end_of_path_distance,
					"status": patrol_path_status,
					"has_patrol_path": ai.has_patrol_path,
					"macro_cursor": ai.has_patrol_path.then_some(ai.macro_command_offset),
				},
				"macro": {
					"remaining_bytes": ai.number_of_remaining_macro_bytes,
					"in_progress": ai.macro_in_progress,
					"started_this_frame": ai.macro_started_in_this_frame,
					"next_rand": ai.next_macro_rand,
					"next_rand_forecasted": ai.next_macro_rand_forecasted,
				},
				"targets": {
					"primary": resolve_ai_handle(ai.primary_target),
					"friend_in_trouble": resolve_ai_handle(ai.friend_in_trouble),
					"detected_body": resolve_ai_handle(ai.detected_body),
					"interesting_object": resolve_ai_handle(ai.interesting_object),
					"antagonist": resolve_ai_handle(ai.antagonist),
					"last_stimulus_actor": ai.last_stimulus_actor.map_or(Value::Null, resolve_ai_handle),
				},
				"timers": {
					"running": ai.timer_is_running, "ring": ai.when_does_timer_ring,
					"macro_running": ai.macro_timer_is_running,
					"macro_ring": ai.when_does_macro_timer_ring,
					"standing_around": ai.standing_around_timer,
				},
			});
			state
				.as_object_mut()
				.expect("parity NPC AI state must be an object")
				.extend(json!({
				"sorrow": ai.sorrow_level,
				"last_stimuli": ai.last_stimulus.map(|stimulus| stimulus as u32),
				"last_stimulus_multiplicities": ai.last_stimulus_multiplicity,
				"group": {
					"is_master": ai.is_master, "master": resolve_ai_handle(ai.master),
					"us": handles(&ai.list_us), "alerted_us": handles(&ai.list_alerted_us),
					"staying_us": handles(&ai.list_staying_us),
				},
				"seek_position": ai_position(ai.seek_position),
				"alert_soldiers_point": ai_position(ai.alert_soldiers_point),
				"first_try": ai.first_try,
				"panic": {
					"center": point2(ai.panic_center_x, ai.panic_center_y),
					"lasting_runs": ai.lasting_panic_runs, "directed": ai.directed_panic,
				},
				"movement_failures": {
					"could_not_reach": ai.couldnt_reachpoint,
					"already_on_point": ai.already_on_point, "already_turned": ai.already_turned,
				},
				"likes_to_sit": ai.likes_to_sit_around, "special_action": ai.special_action,
				"friends_alerted": ai.friends_are_alerted, "stay_at_home": ai.is_stay_at_home,
				"locks": ai.locks_flag_field.bits(), "was_busy": ai.was_busy,
				"stimulus_queue": ai.stimulus_queue.iter().map(stimulus_state).collect::<Vec<_>>(),
				"script_locked": ai.script_locked, "remember_events": ai.remember_events,
				"leave_house_number": ai.leave_house_number,
				"legacy_continuation": {
					"remaining_tequila_gulps": ai.remaining_tequila_gulps,
					"last_hint_actuality": ai.last_hint_actuality,
					"last_hint_subject": ai.last_hint_subject as u32,
					"current_door": ai_door(ai.my_door_index),
					"looking_for_help_because_enemy_seen": ai.looking_for_help_because_enemy_seen,
				},
				"object_memory": {
					"forgotten": handles(&ai.forgotten_objects),
					"desire": resolve_ai_handle(ai.object_of_desire),
					"checkpoint_charly": resolve_ai_handle(ai.checkpoint_charly),
					"synchronize_charly": resolve_ai_handle(ai.synchronize_charly),
				},
				"inside_halt": ai.inside_halt_method,
				"synchronizing_actors": handles(&ai.synchronizing_actors),
				"default_path_flags": ai.default_path_walking_flags.bits(),
				}).as_object().expect("parity NPC AI continuation chunk must be an object").clone());
			state
				.as_object_mut()
				.expect("parity NPC AI state must be an object")
				.extend(json!({
				"current_remark": ai.current_remark as u32,
				"emoticon": {
					"type": ai.current_emoticon_type as u32,
					"expiration": ai.emoticon_expiration_date,
					"has_expiration": ai.emoticon_has_expiration_date,
				},
				"knocked_out_in_money_fight": ai.knocked_out_in_money_fight,
				"got_beggar_trick": ai.got_the_beggar_trick,
				"reconnaissance": {
					"report_type": ai.my_reconnaissance_report.report_type as u32,
					"seek_position": ai_position(ai.my_reconnaissance_report.seek_position),
					"seen_bodies": handles(&ai.my_reconnaissance_report.seen_bodies),
					"charly": resolve_ai_handle(ai.my_reconnaissance_report.charly),
					"charly_seen": ai.my_reconnaissance_report.charly_seen,
				},
				"patrol": {
					"chief": ai.patrol_chief.map_or(Value::Null, entity_ref),
					"active": ai.patrol.iter().copied().map(entity_ref).collect::<Vec<_>>(),
					"missed": ai.missed_patrol_members.iter().copied().map(entity_ref).collect::<Vec<_>>(),
					"theoretical": ai.theoretical_patrol.iter().copied().map(entity_ref).collect::<Vec<_>>(),
					"stopped": ai.patrol_stopped, "direction": ai.patrol_direction,
				},
				}).as_object().expect("parity NPC AI tail chunk must be an object").clone());
			let subclass = match &npc.ai_brain {
				crate::element::AiBrain::Friendly(friendly) => Some(json!({
					"kind": "friendly",
					"fleeing_seen_enemy_counter": friendly.fleeing_seen_enemy_counter,
					"beggar_dont_talk_counter": friendly.beggar_dont_talk_counter,
					"wants_to_talk": friendly.wants_to_talk,
					"last_talk_partner": resolve_ai_handle(friendly.last_talk_partner),
					"can_go_away": friendly.can_go_away,
				})),
				crate::element::AiBrain::Enemy(enemy) => {
					let mut subclass = json!({
					"kind": "enemy",
					"frame_when_missed_charly": enemy.frame_when_missed_charly,
					"frame_when_enemy_detected": enemy.base.frame_when_enemy_detected,
					"fleeing_seen_enemy_counter": enemy.fleeing_seen_enemy_counter,
					"pc_gone_direction": enemy.pc_gone_away_in_this_direction,
					"detected_something_there": ai_position(enemy.detected_something_there),
					"missed_pc": resolve_ai_handle(enemy.missed_pc),
					"last_seek_direction_index": enemy.last_seek_direction_index,
					"beggar_to_examine": resolve_ai_handle(enemy.beggar_to_examine),
					"pc_missed": enemy.pc_missed,
					"task_priorities": {
						"current": enemy.current_task_priority,
						"minimal": enemy.minimal_task_priority,
						"new": enemy.new_task_priority,
					},
					"different_checkpoints": enemy.number_of_different_checkpoints,
					"delta_sorrow": enemy.base.delta_sorrow_level,
					"thirsty": enemy.thirsty,
					"old_life_points": enemy.old_life_points,
					"initial_life_points": enemy.initial_life_points,
					"old_odds": enemy.old_odds,
					"position_change_locked_for_test": enemy.position_change_locked_for_test,
					"heard_nets": handles(&enemy.heard_nets),
					"other_seen_ale": handles(&enemy.other_seen_ale),
					"search_charly_way": enemy.search_charly_way.iter().copied().map(ai_position).collect::<Vec<_>>(),
					"missed_in_action": handles(&enemy.base.missed_in_action),
					"other_bodies_to_examine": handles(&enemy.other_bodies_to_examine),
					"beggars_to_control": handles(&enemy.beggars_to_control),
					"them": handles(&enemy.list_them),
					"ambush_point_array_reset": enemy.ambush_point_array_reset,
					"ambush_point_status": enemy.ambush_point_status.iter().map(|status| *status as u32).collect::<Vec<_>>(),
					"my_seek_points": &enemy.my_seek_points,
					"personal_seek_point_1": enemy.personal_seek_point_1.as_ref().map(|point| seek_point(point)),
					"personal_seek_point_2": enemy.personal_seek_point_2.as_ref().map(|point| seek_point(point)),
					"seek_center": ai_position(enemy.seek_center),
					"actual_seek_point": enemy.actual_seek_point,
					"seek_point_view_directions": &enemy.seek_point_view_directions,
					"positions_of_beggars_to_control": enemy.positions_of_beggars_to_control.iter().copied().map(ai_position).collect::<Vec<_>>(),
					"seek_flags": enemy.seek_flags.bits(),
					"seen_dead_body": enemy.seen_dead_body,
					"seeking_charly": enemy.seeking_charly,
					});
					subclass
						.as_object_mut()
						.expect("parity enemy AI state must be an object")
						.extend(json!({
					"forced_next_battle_decision": enemy.forced_next_battle_decision as u32,
					"reset_battle_decision": enemy.reset_battle_decision,
					"synchronize_index": enemy.base.synchronize_index,
					"initial_view_cone": enemy.base.initial_view_cone as u32,
					"company_number": enemy.company_number,
					"left_combat_neighbour": resolve_ai_handle(enemy.left_combat_neighbour),
					"right_combat_neighbour": resolve_ai_handle(enemy.right_combat_neighbour),
					"attentive": enemy.attentive,
					"will_be_attentive": enemy.will_be_attentive,
					"forced_attentive": enemy.forced_attentive,
					"guarded_pc": enemy.guarded_pc.map_or(Value::Null, |id| entity_ref(EntityId::Pc(id))),
					"tower_guard": enemy.tower_guard,
					"combat_trainer": enemy.combat_trainer,
					"gather_position": ai_position(enemy.gather_position),
					"gather_direction": enemy.gather_direction,
					"gather_position_instructed": enemy.gather_position_instructed,
					"officers_position": ai_position(enemy.officers_position),
					"previous_state": enemy.previous_state,
					"previous_substate": enemy.previous_substate,
					"reported_to_officer": enemy.reported_to_officer,
					"missed_soldier_timer": enemy.missed_soldier_timer,
					"old_money": enemy.old_money,
					"other_seen_money": handles(&enemy.other_seen_money),
					"money_fight_enemies": handles(&enemy.money_fight_enemies),
					"money_fight_victims": handles(&enemy.money_fight_victims),
					"archer_behind_me": resolve_ai_handle(enemy.archer_behind_me),
					"shield_bearer_before_me": resolve_ai_handle(enemy.shield_bearer_before_me),
					"already_seen_bodies": handles(&enemy.already_seen_bodies),
					"my_line_jump": jump_line(enemy.my_line_jump),
					"shield_bearer_direction": enemy.shield_bearer_direction,
					"phalanx_aborted": enemy.phalanx_aborted,
					"changed_to_alert_path": enemy.changed_to_alert_path,
					}).as_object().expect("parity enemy AI continuation must be an object").clone());
					subclass
						.as_object_mut()
						.expect("parity enemy AI state must be an object")
						.extend(json!({
					"shooting_point": enemy.my_shooting_point.map(|(sector_index, point_index)| json!({
						"sector_index": sector_index, "point_index": point_index,
					})),
					"archery_sector": enemy.my_archery_sector,
					"archery_sector_index": enemy.my_archery_sector_index,
					"archery_point_index": enemy.my_archery_point_index.0,
					"archery_point_increment": enemy.my_archery_point_increment,
					"enemy_seen_below": enemy.enemy_seen_below,
					"enemy_had_this_elevation": enemy.enemy_had_this_elevation,
					"known_enemy_strike_commands": [
						known_strike_command(enemy.known_enemy_strike_1),
						known_strike_command(enemy.known_enemy_strike_2),
						known_strike_command(enemy.known_enemy_strike_3),
					],
					"last_stimulus_dispatched_to_patrol": patrol_stimulus(enemy.last_stimulus_dispatched_to_patrol.as_ref()),
					}).as_object().expect("parity enemy archery continuation must be an object").clone());
					Some(subclass)
				},
				crate::element::AiBrain::None => None,
			};
			if let Some(subclass) = subclass {
				state
					.as_object_mut()
					.expect("parity NPC AI state must be an object")
					.insert(
						"subclass".to_owned(),
						subclass,
					);
			}
			Some(state)
		});
        let human_continuation = entity.human_data().map(|human| {
            json!({
                "already_detectable_body": human.already_detectable_body,
                "concussion_healing_timeout": human.concussion_healing_timeout,
                "tiredness": human.tiredness,
                "concussion": human.concussion_of_the_brain,
                "parry_counter": human.parry_counter,
                "detectable_list_index": human.detectable_list_index,
                "invulnerable": human.invulnerable,
                "last_motion_was_step_back": human.last_motion_was_step_back_in_combat,
                "smalltalk_initiative": human.smalltalk_initiative,
                "received_smalltalk_initiative": human.received_smalltalk_initiative,
                "smalltalk_hint": human.smalltalk_hint as u32,
                "smalltalk_hint_opponent": human.smalltalk_hint_opponent.map_or(Value::Null, entity_ref),
                "relative_fighting_ability": human.relative_fighting_ability,
                "hollow_man": human.hollow_man,
                "killed_by_accident": human.killed_by_accident,
                "stuck_under_nets_counter": human.stuck_under_nets_counter,
                "sword_strike_boredom": &human.sword_strike_boredom,
                "carrier": human.carrier.map_or(Value::Null, entity_ref),
                "small_repulsive_radius": human.small_repulsive_radius,
                "hulk": {
                    "running": human.running_hulk, "time": human.time_hulk,
                    "level": human.hulk_level, "direction": human.hulk_direction,
                    "speed": float(human.hulk_speed),
                },
            })
        });
        let human_structure = entity.human_data().map(|human| {
            let opponents = human
                .opponents
                .iter_with_jump_lines()
                .map(|(opponent, line)| json!({
                    "entity": entity_ref(opponent), "jump_line": jump_line(line.map(u32::from)),
                }))
                .collect::<Vec<_>>();
            let repulsive = &human.repulsive_point;
            let shield = &human.shield;
            let plane = |value: &crate::element::HumanPlaneState| json!({
                "a": point3(value.a.x, value.a.y, value.a.z),
                "b": point3(value.b.x, value.b.y, value.b.z),
                "normal": point3(value.normal.x, value.normal.y, value.normal.z),
                "origin": point3(value.origin.x, value.origin.y, value.origin.z),
                "u": point3(value.u.x, value.u.y, value.u.z),
                "v": point3(value.v.x, value.v.y, value.v.z),
                "az": float(value.az), "bz": float(value.bz),
                "dz": float(value.dz), "d": float(value.d),
            });
            let box2_state = |value: crate::element::HumanBoundingBox2State| json!({
                "top_left": point2(value.top_left.x, value.top_left.y),
                "bottom_right": point2(value.bottom_right.x, value.bottom_right.y),
                "bounds_are_set": value.bounds_are_set,
            });
            let sequence_ordinals: std::collections::BTreeMap<_, _> = self
                .inner
                .orders
                .sequence_manager
                .sequences_iter()
                .enumerate()
                .map(|(ordinal, sequence)| (sequence.id, ordinal))
                .collect();
            let sequence_ref = |value: crate::sequence::SequenceElementRef| {
                let sequence = sequence_ordinals.get(&value.sequence_id).copied().unwrap_or_else(|| {
                    panic!("parity human pending shoot points outside sequence manager: {value:?}")
                });
                json!({ "sequence": sequence, "element": value.element_index })
            };
            json!({
                "opponents": opponents,
                "repulsive_point": {
                    "position": point2(repulsive.position.x, repulsive.position.y),
                    "concave": repulsive.concave,
                    "limit_left": point2(repulsive.limit_left.x, repulsive.limit_left.y),
                    "limit_right": point2(repulsive.limit_right.x, repulsive.limit_right.y),
                    "action_radius": float(repulsive.action_radius),
                    "force_a": float(repulsive.force_a), "force_b": float(repulsive.force_b),
                    "radius": float(repulsive.radius), "id": repulsive.id,
                    "affects_pcs": repulsive.affects_pcs,
                    "affects_soldiers": repulsive.affects_soldiers,
                    "affects_civilians": repulsive.affects_civilians,
                    "affects_animals": repulsive.affects_animals,
                },
                "building": sector(human.building_sector),
                "shield": {
                    "points": shield.points.iter().map(|value| json!({
                        "obstacle": value.obstacle.map(float),
                        "polygon": point2(value.polygon.x, value.polygon.y),
                    })).collect::<Vec<_>>(),
                    "top_plane": plane(&shield.top_plane),
                    "bottom_plane": plane(&shield.bottom_plane),
                    "box_3d": shield.box_3d.map(float),
                    "ground_box": box2_state(shield.ground_box),
                    "screen_box": box2_state(shield.screen_box),
                    "on_ground": shield.on_ground,
                },
                "sword_sweep": {
                    "victims": human.sword_sweep.victims.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "initial_angle": float(human.sword_sweep.initial_angle),
                    "current_angle": float(human.sword_sweep.current_angle),
                    "final_angle": float(human.sword_sweep.final_angle),
                },
                "pending_shoots": human.pending_shoots.iter().copied().map(sequence_ref).collect::<Vec<_>>(),
            })
        });
        let pc_core = entity.pc_data().map(|pc| {
            const ACTIONS: usize = 3;
            assert_eq!(
                pc.disabled_actions.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} permanent action flags, expected {ACTIONS}",
                pc.disabled_actions.len()
            );
            assert_eq!(
                pc.disabled_actions_temp.len(),
                ACTIONS,
                "PC {id:?} parity projection has {} temporary action flags, expected {ACTIONS}",
                pc.disabled_actions_temp.len()
            );
            let campaign_description_index = pc.campaign_description_index.unwrap_or_else(|| {
                panic!("PC {id:?} parity projection has no campaign description index")
            });
            json!({
                "work_icon": pc.work_icon as u32,
                "campaign_description_index": campaign_description_index,
                "playable": pc.playable,
                "beam_me_index": pc.beam_me_index,
                "already_selected": pc.already_selected,
                "belt_seen": pc.belt_seen,
                "feet_seen": pc.feet_seen,
                "head_seen": pc.head_seen,
                "immortal": pc.immortal,
                "fried_psykokwack": pc.fried_psykokwack,
                "list_index": pc.list_index,
                "teleport_counter": pc.teleport_counter,
                "current_action": pc.current_action as u32,
                "saved_action": pc.saved_action as u32,
                "disabled_actions": pc.disabled_actions,
                "disabled_actions_temp": pc.disabled_actions_temp,
                "position_before_teleport": point2(
                    pc.position_before_teleport.x,
                    pc.position_before_teleport.y,
                ),
            })
        });
        let pc_qa = entity.pc_data().map(|pc| {
            const QA_SLOTS: usize = crate::macro_store::NUMBER_OF_QA_MEMORY;
            for (name, length) in [
                ("types", pc.quick_action_types.len()),
                ("actions", pc.quick_action_sequences.len()),
                ("seeks", pc.quick_seek_sequences.len()),
                ("special-counts", pc.quick_action_special_counts.len()),
                ("buttons", pc.quick_action_buttons.len()),
                ("interactors", pc.quick_action_interactors.len()),
                ("titbits", pc.titbits.len()),
            ] {
                assert_eq!(
                    length, QA_SLOTS,
                    "PC {id:?} parity projection has {length} {name}, expected {QA_SLOTS}"
                );
            }
            (0..QA_SLOTS)
                .map(|slot| {
                    json!({
                        "special_count": pc.quick_action_special_counts[slot],
                        "quickito": pc.quick_action_types[slot] as u32,
                        "titbit": pc.titbits[slot],
                        "button": pc.quick_action_buttons[slot],
                        "interactor": pc.quick_action_interactors[slot].map_or(Value::Null, entity_ref),
                        "action_size": pc.quick_action_sequences[slot].as_ref().map(crate::sequence::Sequence::len),
                        "seek_size": pc.quick_seek_sequences[slot].as_ref().map(crate::sequence::Sequence::len),
                    })
                })
                .collect::<Vec<_>>()
        });
        let pc_interface = entity.pc_data().map(|pc| {
            json!({
                "playable": pc.playable,
                "displayed": !pc.interface_hidden,
            })
        });
        let pc_portrait = entity.pc_data().map(|pc| {
            let profile = assets
                .profile_manager
                .get_character(pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {id:?} portrait has missing profile {}",
                        pc.profile_index
                    )
                });
            let description = self
                .inner
                .pc_description_for_pc_data(pc)
                .unwrap_or_else(|| panic!("PC {id:?} portrait has no campaign description"));
            let quantities = profile
                .actions
                .map(|action| description.status.get_ammo(action));
            json!({
                "quantities": quantities,
                "two_buttons_mode": profile.actions[2] == crate::profiles::Action::NoAction,
                "displayed": !pc.interface_hidden,
                "burned": pc.portrait.burned,
                "open": pc.portrait.open,
                "life_level": float(f32::from(pc.life_points)),
                "trumpet_enabled": pc.trumpet_enabled,
                "quick_icons": pc.portrait.quick_icons.iter().map(|icon| json!({
                    "titbit": icon.titbit_id, "running": icon.running,
                })).collect::<Vec<_>>(),
            })
        });
        let pc_tail = entity.pc_data().map(|pc| {
            json!({
                "carried": pc.carried.map_or(Value::Null, entity_ref),
                "carried_posture": pc.carried_posture,
                "shield_danger_point": point3(
                    pc.shield_danger_point.x,
                    pc.shield_danger_point.y,
                    pc.shield_danger_point.z,
                ),
                "shield_protected": pc.shield_protected.map_or(Value::Null, entity_ref),
                "shield_protector": pc.shield_protector.map_or(Value::Null, entity_ref),
                "guard": pc.guard.map_or(Value::Null, entity_ref),
                "time_till_reinforcement": pc.time_till_reinforcement,
                "last_ammo_dropping_position": point2(
                    pc.last_ammo_dropping_position.x,
                    pc.last_ammo_dropping_position.y,
                ),
                "last_dropped_ammo": pc.last_dropped_ammo.map_or(Value::Null, entity_ref),
                "update_last_dropped_ammo": pc.update_last_dropped_ammo,
                "last_dropping_direction": pc.last_dropping_direction,
            })
        });
        let subtype = if entity.element_data().active {
            match entity {
                crate::element::Entity::Target(target) => Some(json!({
                    "kind": "target",
                    "animation": target.target.animation as u32,
                    "progression": target.target.progression,
                    "linked_fx": target.target.linked_fx.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "force_display": target.fx.force_display,
                    "restore_background": target.fx.restore_background,
                })),
                crate::element::Entity::Scroll(scroll) => Some(json!({
                    "kind": "scroll",
                    "status": self.inner.scroll_status(id) as i32,
                    "script_hourglass_timeout": scroll.script_hourglass_timeout,
                })),
                crate::element::Entity::Net(net) => Some(json!({
                    "kind": "net",
                    "projectile": projectile_state(&net.projectile),
                    "victims": net.net.victims.iter().copied().map(entity_ref).collect::<Vec<_>>(),
                    "time_till_unfolding": net.net.time_till_unfolding,
                    "crumpled": net.net.crumpled,
                    "was_flying": net.net.was_flying,
                })),
                crate::element::Entity::Projectile(projectile) => {
                    use crate::element_kinds::ObjectType;
                    let common = projectile_state(&projectile.projectile);
                    Some(match projectile.object.object_type {
                        ObjectType::Arrow => json!({
                            "kind": "arrow", "projectile": common,
                            "bow_profile": projectile.projectile.arrow_bow_profile.flatten(),
                            "flat_shot": projectile.projectile.arrow_flat_shot,
                            "falling": projectile.projectile.falling,
                            "falling_direction": projectile.projectile.falling_direction,
                            "last_sector": projectile.projectile.last_orientation_sector,
                            "last_azimuth": projectile.projectile.last_orientation_azimuth,
                            "play_impact": projectile.projectile.arrow_play_impact,
                        }),
                        ObjectType::Purse => json!({
                            "kind": "purse", "projectile": common,
                            "number_of_coins": projectile.projectile.purse.number_of_coins,
                        }),
                        ObjectType::Coin => json!({
                            "kind": "coin", "projectile": common,
                            "source_purse": projectile.projectile.purse.source_purse.map_or(Value::Null, entity_ref),
                        }),
                        ObjectType::Wasp => json!({
                            "kind": "wasp",
                            "nest": projectile.projectile.wasp.source_nest.map_or(Value::Null, entity_ref),
                            "victim": projectile.projectile.wasp.victim.map_or(Value::Null, entity_ref),
                            "stinging": projectile.projectile.wasp.stinging,
                            "timeout": projectile.projectile.wasp.timeout,
                            "movement": point3(projectile.projectile.wasp.movement.x,
                                projectile.projectile.wasp.movement.y, projectile.projectile.wasp.movement.z),
                        }),
                        ObjectType::WaspNest | ObjectType::BonusWaspNest => json!({
                            "kind": "wasp_nest", "projectile": common,
                            "flying_wasp_count": projectile.projectile.wasp.flying_wasp_count,
                        }),
                        _ => json!({ "kind": "projectile", "projectile": common }),
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        let mut result = json!({
            "position": position_state,
            "sprite": sprite_state,
        });
        if let Some(subtype) = subtype {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("subtype".to_owned(), subtype);
        }
        if let Some(npc_ai) = npc_ai {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("npc_ai".to_owned(), npc_ai);
        }
        if let Some(human_continuation) = human_continuation {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("human_continuation".to_owned(), human_continuation);
        }
        if let Some(human_structure) = human_structure {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("human_structure".to_owned(), human_structure);
        }
        if let Some(pc_tail) = pc_tail {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("pc_tail".to_owned(), pc_tail);
        }
        if let Some(pc_core) = pc_core {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("pc_core".to_owned(), pc_core);
        }
        if let Some(pc_qa) = pc_qa {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("pc_qa".to_owned(), Value::Array(pc_qa));
        }
        if let Some(pc_interface) = pc_interface {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("pc_interface".to_owned(), pc_interface);
        }
        if let Some(pc_portrait) = pc_portrait {
            result
                .as_object_mut()
                .expect("parity entity runtime must be an object")
                .insert("pc_portrait".to_owned(), pc_portrait);
        }
        result
    }
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
            next_creation_order: self.inner.world.next_original_creation_order,
            chorus_timer: self.inner.control.chorus_timer,
            force_check: self.inner.script_domains.mission_ui.force_check,
            men_to_blazon_conversion: self
                .inner
                .script_domains
                .mission_ui
                .men_to_blazon_conversion_mode,
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

    /// Exact serialized `RHGame` mission/controller latches. Host widgets
    /// mirror these values but do not own their authoritative state.
    #[doc(hidden)]
    pub fn parity_game_ui_state(&self) -> serde_json::Value {
        let ui = &self.inner.script_domains.mission_ui;
        serde_json::json!({
            "campaign_map": ui.campaign_map,
            "campaign_map_displayed": ui.campaign_map_displayed,
            "post_initialized": ui.game_post_initialized,
            "start_mission_disabled_temp": ui.start_mission_disabled_temp,
            "quit_mission_disabled_temp": ui.quit_mission_disabled_temp,
            "start_mission_enabled": ui.start_mission_enabled,
            "quit_mission_enabled": ui.quit_mission_enabled,
        })
    }

    /// Serialized messenger controller state that remains gameplay-visible.
    #[doc(hidden)]
    pub fn parity_messenger_controller_state(&self) -> serde_json::Value {
        serde_json::json!({
            "view_locked": self.inner.players.view_locked,
            "selected_action": self.inner.players.seats[0].selected_action as u32,
        })
    }

    /// Serialized engine-global two-click shield controller. This is separate
    /// from each PC's active shield links in `pc_tail`.
    #[doc(hidden)]
    pub fn parity_shield_controller_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = |id: EntityId| {
            let kind = match id.kind() {
                crate::element::EntityIdKind::Pc => "pc",
                other => panic!("shield controller protects non-PC entity {other:?}"),
            };
            json!({ "kind": kind, "index": id.index() })
        };
        let shield = &self.inner.world.shield;
        json!({
            "is_protected": shield.is_protected,
            "protected_pc": shield.protected_pc.map(&entity).unwrap_or(Value::Null),
            "danger_point": {
                "x": { "bits": shield.danger_point.x.to_bits() },
                "y": { "bits": shield.danger_point.y.to_bits() },
                "z": { "bits": shield.danger_point.z.to_bits() },
            },
        })
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
                            "destination_3d": point3(
                                order.destination_3d[0],
                                order.destination_3d[1],
                                order.destination_3d[2],
                            ),
                            "flight_vector": point(
                                order.flight_vector[0],
                                order.flight_vector[1],
                            ),
                            "tolerance": float(order.tolerance),
                            "apply_transition": order.apply_transition_at_this_point,
                            "reverse": order.reverse,
                            "compute_direction": order.compute_direction,
                            "can_fly": order.can_fly,
                            "lock_ai": order.lock_ai,
                            "transition": order.transition,
                            "done": order.done,
                            "id": order.order_id.get() - 1,
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
            "next_order_id": self.inner.orders.next_order_id - 1,
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

    /// Sparse, serialized sound-source manager state. Host channels and
    /// backend playback queues are deliberately absent; these are the source
    /// fields that survive Original save/load and feed later simulation.
    #[doc(hidden)]
    pub fn parity_sound_sources_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let float = |value: f32| json!({ "bits": value.to_bits(), "value": value });
        let sources = &self.inner.feedback.sound_sim.sources;
        let mut result = Vec::with_capacity(sources.num_sources());
        for index in 0..sources.num_sources() {
            let Some(source) = sources.get(index) else {
                result.push(Value::Null);
                continue;
            };
            let kind = match source.source_kind {
                crate::sound_source::SoundSourceKind::Single => 0,
                crate::sound_source::SoundSourceKind::Looped => 1,
                crate::sound_source::SoundSourceKind::Delayed => 2,
                crate::sound_source::SoundSourceKind::Volatile => 3,
            };
            let altitude = match source.altitude {
                crate::sound_geometry::SoundSourceAltitude::Ground => 0,
                crate::sound_geometry::SoundSourceAltitude::Middle => 1,
                crate::sound_geometry::SoundSourceAltitude::Top => 2,
                crate::sound_geometry::SoundSourceAltitude::NoAltitude => 3,
            };
            result.push(json!({
                "kind": kind,
                "id": source.id,
                "global": source.is_global,
                "inner_distance": source.inner_distance,
                "outer_distance": source.outer_distance,
                "noise_covering_distance": source.noise_covering_distance,
                "inner_volume": source.inner_volume,
                "outer_volume": source.outer_volume,
                "shape": source.shape.iter().map(|point| json!({
                    "x": float(point.x), "y": float(point.y)
                })).collect::<Vec<_>>(),
                "altitude": altitude,
                "min_delay": source.min_delay,
                "max_delay": source.max_delay,
                "delay_stepping": source.delay_stepping,
                "timer": source.timer,
                "active": source.active,
            }));
        }
        Value::Array(result)
    }

    /// Ordered deterministic source-completion deadlines. Looped sources have
    /// no completion entry; Single, Volatile, and Delayed sources retain the
    /// order in which Original queued their pending playback records.
    #[doc(hidden)]
    pub fn parity_sound_completion_frontier_state(&self) -> serde_json::Value {
        use serde_json::json;

        serde_json::Value::Array(
            self.inner
                .feedback
                .sound_sim
                .playing_sources
                .iter()
                .map(|playing| {
                    if self
                        .inner
                        .feedback
                        .sound_sim
                        .sources
                        .get(playing.source_index as usize)
                        .is_none()
                    {
                        panic!(
                            "sound completion frontier references missing source {}",
                            playing.source_index
                        );
                    }
                    json!({
                        "source_index": playing.source_index,
                        "finish_frame": playing.finish_frame,
                    })
                })
                .collect(),
        )
    }

    /// Serialized global AI-manager state. Mission-static seek/archery
    /// geometry is intentionally absent; Original persists only these ordered
    /// mutable statuses, reservations, counters, alerts, and saved RNG seed.
    #[doc(hidden)]
    pub fn parity_ai_global_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

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
        let global = &self.inner.ai.global;
        json!({
            "stupid_soldiers_cheat": global.stupid_soldiers_cheat,
            "seek_points": global.seek_points.iter().map(|point| json!({
                "frame_when_full_interest": point.frame_when_full_interest,
                "last_calculated_interest": point.last_calculated_interest,
                "locked": point.locked,
            })).collect::<Vec<_>>(),
            "archery_sectors": global.archery_sectors.iter().map(|sector| json!({
                "num_owners": sector.num_owners,
                "point_owners": sector.points.iter().map(|point| point.owner
                    .map(&entity).unwrap_or(Value::Null)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "green_alert_soldiers": global.green_alert_soldiers,
            "yellow_alert_soldiers": global.yellow_alert_soldiers,
            "red_alert_soldiers": global.red_alert_soldiers,
            "overall_alert_status": global.overall_alert_status as u32,
            "overall_villain_alert_status": global.overall_villain_alert_status as u32,
            "saved_random_seed": global.saved_random_seed,
            "forbidden_remarks": global.forbidden_remarks.iter().map(|entry| json!({
                "remark": entry.remark as u32,
                "flags": entry.flags,
                "speech_id": entry.speech_id,
                // This is deliberately the stored scalar, not a normalized
                // entity reference. Original stores GetCreationOrder() here;
                // parity must expose any slot-vs-creation-order divergence.
                "guy_index": entry.guy_index,
                "bad_guy": entry.bad_guy,
                "forbidden_till_frame": entry.forbidden_till_frame,
            })).collect::<Vec<_>>(),
            "current_speech_variant": global.current_speech_variant,
        })
    }

    /// Exact `RHEngine::marrayActorsPC` order. The portrait bar has a
    /// different priority-sorted owner and must not stand in for gameplay
    /// loops that walk Original's PC registry.
    #[doc(hidden)]
    pub fn parity_pc_registry_state(&self) -> serde_json::Value {
        use serde_json::json;

        serde_json::Value::Array(
            self.inner
                .world
                .original_pc_registry_ids
                .iter()
                .map(|id| {
                    let kind = match id.kind() {
                        crate::element::EntityIdKind::Pc => "pc",
                        other => panic!("Original PC registry contains non-PC entity {other:?}"),
                    };
                    json!({ "kind": kind, "index": id.index() })
                })
                .collect(),
        )
    }

    /// Engine-owned roots serialized outside the element/sequence managers.
    /// References use the same semantic entity and manager-ordinal sequence
    /// forms as the rest of the parity snapshot.
    #[doc(hidden)]
    pub fn parity_engine_runtime_roots_state(
        &self,
        menu_text: &dyn crate::sherwood_stat::MenuTextLookup,
    ) -> serde_json::Value {
        use serde_json::{Value, json};

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
        let reference = |value: crate::sequence::SequenceElementRef| {
            let sequence = sequence_ordinals
                .get(&value.sequence_id)
                .copied()
                .unwrap_or_else(|| {
                    panic!(
                        "parity runtime root points outside sequence manager: {:?}/{}",
                        value.sequence_id, value.element_index
                    )
                });
            json!({ "sequence": sequence, "element": value.element_index })
        };
        let stat = &self.inner.mission_domain.mission_stat;
        let pc_names = stat
            .pc_names
            .iter()
            .map(|name| {
                if let Some(slot) = name.name_override {
                    let resolved = menu_text.get(slot.menu_text_id());
                    if !resolved.is_empty() {
                        return resolved;
                    }
                }
                name.fallback.clone()
            })
            .collect::<Vec<_>>();

        json!({
            "timer_elements": self.inner.orders.timer_elements.iter().map(|timer| json!({
                "element": reference(timer.element_ref), "remaining": timer.remaining,
            })).collect::<Vec<_>>(),
            "camera_sequence": self.inner.feedback.cutscene_camera.sequence_element
                .map(&reference).unwrap_or(Value::Null),
            "dead_pc": self.inner.mission_domain.dead_pc.map(&entity).unwrap_or(Value::Null),
            "mission_stat": {
                "collected_money": stat.collected_money,
                "bonus_money": stat.bonus_money,
                "soldier_money": stat.soldier_money,
                "living_soldier_count": stat.living_soldier_count,
                "total_soldier_count": stat.total_soldier_count,
                "new_peasant_count": stat.new_peasant_count,
                "killed_peasant_count": stat.killed_peasant_count,
                "killed_allied_count": stat.killed_allied_count,
                "added_score": stat.added_score,
                "pc_names": pc_names,
            },
            "user_locked": self.inner.players.user_locked,
            "selection_before_user_lock": self.inner.players.selection_before_user_lock
                .iter().copied().map(&entity).collect::<Vec<_>>(),
            "follow_element": self.inner.players.seats[0].follow_element
                .map(&entity).unwrap_or(Value::Null),
        })
    }

    /// Mutable patch, gate, and door-sector state in canonical mission-table
    /// order. Static geometry and patch configuration come from level data and
    /// are deliberately not duplicated.
    #[doc(hidden)]
    pub fn parity_world_interactables_state(&self, assets: &LevelAssets) -> serde_json::Value {
        use serde_json::{Value, json};

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
        let interactables = &self.inner.script_domains.interactables;
        let patches = interactables
            .patches
            .iter()
            .map(|patch| {
                json!({
                    "active": patch.active,
                    "locked": patch.locked,
                    "occupants": patch.occupants.iter().map(|occupant| {
                        let id = self.inner.entity_id_for_index(occupant.0).unwrap_or_else(|| {
                            panic!("parity patch occupant references missing entity {}", occupant.0)
                        });
                        entity(id)
                    }).collect::<Vec<_>>(),
                    "applied": patch.applied,
                    "in_transition": patch.in_transition,
                })
            })
            .collect::<Vec<_>>();
        let doors = interactables
            .doors
            .iter()
            .map(|door| match door.gate_type {
                crate::gate::GateType::Door => json!({
                    "kind": "door",
                    "active": door.active,
                    "locked_pc": door.locked_pc,
                    "locked_npc_villain": door.locked_npc_villain,
                    "locked_npc_civilian": door.locked_npc_civilian,
                    "unlockable": door.unlockable,
                    "locked_pc_after_patch": door.locked_pc_after_patch,
                    "locked_npc_villain_after_patch": door.locked_npc_villain_after_patch,
                    "locked_npc_civilian_after_patch": door.locked_npc_civilian_after_patch,
                    "unlockable_after_patch": door.unlockable_after_patch,
                    "special_authorisation_pc": door.special_authorisation_pc,
                    "authorised_pc_direct": door.authorised_pc_direct,
                    "authorised_pc_indirect": door.authorised_pc_indirect,
                }),
                crate::gate::GateType::Jump => json!({
                    "kind": "jump", "active": door.active,
                }),
                crate::gate::GateType::None => json!({
                    "kind": "gate", "active": door.active,
                }),
            })
            .collect::<Vec<_>>();
        let grid = &self.inner.world.fast_grid;
        let sector_doors = grid
            .level
            .sectors
            .iter()
            .enumerate()
            .filter(|(_, sector)| sector.sector_type.is_door())
            .map(|(index, sector)| {
                let active = *grid.sector_active.get(index).unwrap_or_else(|| {
                    panic!("parity door sector {index} has no active-state slot")
                });
                json!({ "sector": sector.sector_number.get(), "active": active })
            })
            .collect::<Vec<_>>();

        let lifts = grid
            .level
            .sectors
            .iter()
            .enumerate()
            .filter(|(_, sector)| sector.sector_type.is_lift())
            .map(|(index, sector)| {
                let state = grid
                    .lift_state
                    .get(&(index as u32))
                    .copied()
                    .unwrap_or_default();
                json!({
                    "sector": sector.sector_number.get(),
                    "occupants_pc": state.occupants_pc,
                    "occupants": state.occupants,
                    "occupied_upwards": state.occupied_upwards,
                    "occupied_downwards": state.occupied_downwards,
                    "wait_time": state.wait_time,
                })
            })
            .collect::<Vec<_>>();

        let buildings = &self.inner.script_domains.buildings;
        if buildings.occupants.len() != buildings.arrow_reserves.len() {
            panic!(
                "building occupant table length {} differs from arrow-reserve table length {}",
                buildings.occupants.len(),
                buildings.arrow_reserves.len()
            );
        }
        let building_state = buildings
            .occupants
            .iter()
            .zip(&buildings.arrow_reserves)
            .map(|(occupants, &arrow_reserve)| {
                json!({
                    "occupants": occupants.iter().map(|&handle| {
                        let id = self.inner.entity_id_for_actor_handle(handle).unwrap_or_else(|| {
                            panic!("parity building occupant has invalid actor handle {handle}")
                        });
                        entity(id)
                    }).collect::<Vec<_>>(),
                    "arrow_reserve": arrow_reserve,
                })
            })
            .collect::<Vec<_>>();

        let zones = &self.inner.script_domains.zones.scripts;
        if zones.len() != assets.scripts.zone_grid_indices.len() {
            panic!(
                "script-zone runtime length {} differs from topology length {}",
                zones.len(),
                assets.scripts.zone_grid_indices.len()
            );
        }
        let script_zones = zones
            .iter()
            .zip(assets.scripts.zone_grid_indices.iter().copied())
            .map(|(zone, grid_index)| {
                let grid_apex = grid
                    .sector_type(grid_index)
                    .contains(crate::sector::SectorType::APEX);
                if grid_apex != zone.transformed_to_apex {
                    panic!(
                        "script-zone apex state disagrees with sector overlay at grid index {grid_index}"
                    );
                }
                json!({
                    "occupants": zone.occupant_indices.iter().copied().map(&entity)
                        .collect::<Vec<_>>(),
                    "transformed_to_apex": zone.transformed_to_apex,
                    "max_apex_height": zone.transformed_to_apex.then(|| {
                        json!({
                            "bits": zone.max_throwing_apex_height.to_bits(),
                            "value": zone.max_throwing_apex_height,
                        })
                    }).unwrap_or(Value::Null),
                })
            })
            .collect::<Vec<_>>();

        json!({
            "patches": patches,
            "doors": doors,
            "sector_doors": sector_doors,
            "lifts": lifts,
            "buildings": building_state,
            "script_zones": script_zones,
        })
    }

    /// Ordered script-created repulsive points plus Original's process-global
    /// next-ID counter. Mission-authored geometry is reconstructed from level
    /// data and is not duplicated here.
    #[doc(hidden)]
    pub fn parity_repulsive_points_state(&self) -> serde_json::Value {
        use serde_json::json;

        let float = |value: f32| json!({ "bits": value.to_bits(), "value": value });
        let points = self
            .inner
            .ai
            .global
            .repulsive_points
            .iter()
            .map(|point| {
                let id = u32::try_from(point.id).unwrap_or_else(|_| {
                    panic!("parity repulsive point has negative ID {}", point.id)
                });
                json!({
                    "position": {
                        "x": float(point.position.x),
                        "y": float(point.position.y),
                    },
                    "concave": point.concave,
                    "limit_left": {
                        "x": float(point.limit_left.x),
                        "y": float(point.limit_left.y),
                    },
                    "limit_right": {
                        "x": float(point.limit_right.x),
                        "y": float(point.limit_right.y),
                    },
                    "action_radius": float(point.action_radius),
                    "force_a": float(point.force_a),
                    "force_b": float(point.force_b),
                    "radius": float(point.radius),
                    "id": id,
                    "affects_pcs": point.flags & 1 != 0,
                    "affects_soldiers": point.flags & 2 != 0,
                    "affects_civilians": point.flags & 4 != 0,
                    "affects_animals": point.flags & 8 != 0,
                    "layer": point.position.level,
                })
            })
            .collect::<Vec<_>>();

        json!({
            "next_id": self.inner.world.original_repulsive_point_counter,
            "points": points,
        })
    }

    /// Original-serialized titbit-manager state. Render-only manager counters
    /// are excluded, but every live titbit field is retained because existence,
    /// lifetime, phase, and manager links participate in game logic.
    #[doc(hidden)]
    pub fn parity_titbit_manager_state(&self) -> serde_json::Value {
        use serde_json::{Value, json};

        let entity = |handle: crate::titbit::ElementHandle| {
            if !handle.is_valid() {
                return Value::Null;
            }
            let id = self
                .inner
                .entity_id_for_index(handle.0)
                .unwrap_or_else(|| panic!("parity titbit references missing entity {}", handle.0));
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
        let float = |value: f32| json!({ "bits": value.to_bits(), "value": value });
        let manager = &self.inner.feedback.titbit_manager;
        json!({
            "current_id": manager.parity_current_id(),
            "titbits": manager.titbits().iter().map(|titbit| json!({
                "kind": titbit.kind as u32,
                "frame_count": titbit.frame_count,
                "sprite_frame": titbit.sprite_frame,
                "sprite_row": titbit.sprite_row,
                "phase": titbit.phase,
                "display_order": float(titbit.display_order),
                "layer": titbit.layer,
                "blinking": titbit.blinking,
                "id": titbit.id,
                "element_supplier": entity(titbit.element_supplier),
                "element_manager": entity(titbit.element_manager),
                "position": {
                    "x": float(titbit.position.x),
                    "y": float(titbit.position.y),
                    "z": float(titbit.position.z),
                },
            })).collect::<Vec<_>>(),
        })
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

    /// Supply one frame's captured results for Original's undefined stale
    /// sprite action-point read. This is a parity-tool boundary, analogous to
    /// the captured Original RNG stream; live simulation leaves it empty.
    pub fn set_original_impossible_action_done_deadlines(
        &mut self,
        deadlines: impl IntoIterator<Item = (u32, u32, i16)>,
    ) {
        let mut captured = std::collections::BTreeMap::new();
        for (proposer_creation_order, target_creation_order, deadline) in deadlines {
            captured
                .entry((proposer_creation_order, target_creation_order))
                .or_insert_with(std::collections::VecDeque::new)
                .push_back(deadline);
        }
        self.inner.control.original_impossible_action_done_deadlines = captured;
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

    fn apply_external_action(
        &mut self,
        assets: &LevelAssets,
        dev: &mut DevState,
        action: ExternalAction,
    ) -> ExternalActionResult {
        match action {
            ExternalAction::Native {
                name,
                args,
                this_actor,
            } => ExternalActionResult::Native(
                self.call_external_native_with_this(assets, &name, &args, this_actor),
            ),
            ExternalAction::CheatString {
                input,
                mut selected_view_element,
            } => {
                let response =
                    self.run_cheat_string(assets, dev, &mut selected_view_element, &input);
                ExternalActionResult::CheatString {
                    response: FrameConsoleResponse::from(response),
                    selected_view_element,
                }
            }
            ExternalAction::ConsoleCommand {
                input,
                mut selected_view_element,
            } => {
                let response =
                    self.run_console_command(assets, dev, &mut selected_view_element, &input);
                ExternalActionResult::ConsoleCommand {
                    response: FrameConsoleResponse::from(response),
                    selected_view_element,
                }
            }
            ExternalAction::SimpleMessage { message } => {
                self.inner.send_simple_message(message);
                ExternalActionResult::SimpleMessage
            }
            ExternalAction::EzekielInstakill { target } => {
                ExternalActionResult::EzekielInstakill(self.inner.try_ezekiel_instakill(target))
            }
            ExternalAction::ReplaceCampaign { campaign } => {
                self.inner.replace_campaign(campaign);
                ExternalActionResult::ReplaceCampaign
            }
        }
    }

    /// Advance one authoritative host-admitted engine frame.
    ///
    /// This is the migration target for drivers that currently call external
    /// replay hooks, `apply_commands`, and `perform_hourglass` separately. It
    /// preserves the Original's boundary ordering:
    ///
    /// 1. between-frame director/sound facts,
    /// 2. admitted pre-hourglass host actions,
    /// 3. resolved player commands in recorded order,
    /// 4. the explicitly gated `PerformHourglass`,
    /// 5. admitted post-hourglass developer/native actions,
    /// 6. post-hourglass commands,
    /// 7. the optional one-shot `PostInitialize` stage,
    /// 8. side-effect drain and post-frame state hash.
    ///
    /// `display`, `input`, and `dev` remain explicit host-owned adapter state
    /// while command handlers are incrementally disentangled from UI scratch;
    /// none of them is part of [`SimulationFrameInput`]. Rendering, widgets,
    /// audio playback remain outside this call at their Original boundaries.
    /// Graphical play can cross `PostInitialize` with a second no-hourglass
    /// admission after presentation without exposing a separate mutation API.
    pub fn advance_frame(
        &mut self,
        display: &mut super::HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        dev: &mut DevState,
        frame: SimulationFrameInput,
    ) -> Result<SimulationFrameOutput, FrameAdvanceError> {
        self.require_live_campaign("advancing a simulation frame");

        let frame_before = self.inner.control.frame_counter;
        let SimulationFrameInput {
            external_facts,
            external_actions,
            commands,
            post_external_actions,
            post_commands,
            run_hourglass,
            simulation_body_allowed,
            run_post_initialize,
        } = frame;

        // Facts are fallible authoritative inputs. Stage the complete ordered
        // prefix so a corrupt later completion or sound resolution cannot
        // leave earlier director/sound mutations partially committed.
        if !external_facts.is_empty() {
            let mut staged_inner = self.inner.clone();
            let mut staged_display = display.clone();
            Self::apply_frame_external_facts(
                &mut staged_inner,
                &mut staged_display,
                assets,
                external_facts,
            )?;
            self.inner = staged_inner;
            *display = staged_display;
        }

        let mut external_action_results = external_actions
            .into_iter()
            .map(|action| self.apply_external_action(assets, dev, action))
            .collect::<Vec<_>>();

        let commands: Vec<PlayerInput> = commands.into_iter().map(Into::into).collect();
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_commands(&sim, display, input, assets, &commands);

        let side_effects = if run_hourglass {
            self.inner.perform_hourglass_with_body_gate(
                display,
                assets,
                dev,
                simulation_body_allowed,
            )
        } else {
            let mut effects = SideEffects::default();
            effects.code = crate::game_operation::GameCode::LevelInProgress;
            effects
        };

        external_action_results.extend(
            post_external_actions
                .into_iter()
                .map(|action| self.apply_external_action(assets, dev, action)),
        );

        let post_commands: Vec<PlayerInput> = post_commands.into_iter().map(Into::into).collect();
        let sim = self.inner.control.simulation_context();
        self.inner
            .apply_commands(&sim, display, input, assets, &post_commands);

        let post_initialize_events = run_post_initialize
            .then(|| self.inner.perform_post_initialize(display, assets))
            .flatten()
            .map(SimEvents::from);
        let frame_after = self.inner.control.frame_counter;
        let state_hash = crate::replay::state_hash(&self.inner);

        Ok(SimulationFrameOutput {
            frame_before,
            frame_after,
            hourglass_ran: run_hourglass,
            events: SimEvents::from(side_effects),
            post_initialize_events,
            external_action_results,
            state_hash,
        })
    }

    fn apply_frame_external_facts(
        inner: &mut EngineInner,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        external_facts: ExternalFacts,
    ) -> Result<(), FrameAdvanceError> {
        for (index, completion) in external_facts.director_completions.into_iter().enumerate() {
            inner
                .apply_external_director_completion(completion, display, assets)
                .map_err(|reason| FrameAdvanceError::DirectorCompletionRejected {
                    index,
                    completion,
                    reason,
                })?;
        }
        if let Some(sound_boundary) = external_facts.sound_boundary {
            let policy = sound_boundary.policy;
            inner
                .try_queue_resolved_exclamations(
                    sound_boundary.resolutions,
                    policy == SoundBoundaryPolicy::Replay,
                )
                .map_err(|reason| FrameAdvanceError::SoundBoundaryRejected { policy, reason })?;
            let sim = inner.control.simulation_context();
            inner
                .hourglass_phase_sound_boundary(&sim, assets)
                .map_err(|reason| FrameAdvanceError::SoundBoundaryRejected { policy, reason })?;
        }
        Ok(())
    }

    /// Select whether recorded between-frame director events own completion
    /// timing for camera sequence elements.
    pub fn set_external_director_completion_replay(&mut self, enabled: bool) {
        self.inner.set_external_director_completion_replay(enabled);
    }

    /// Apply one recorded director completion at the pre-Hourglass boundary.
    ///
    /// This validates the currently latched sequence command, terminates it,
    /// and synchronously runs immediate successors before returning.
    #[cfg(test)]
    pub(crate) fn apply_external_director_completion(
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
    #[cfg(test)]
    pub(crate) fn perform_hourglass(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> SideEffects {
        self.require_live_campaign("performing an engine tick");
        self.inner.perform_hourglass(display, assets, dev)
    }

    /// Apply a batch of player commands, as used by the replay driver
    /// and the rollback checker.
    #[cfg(test)]
    pub(crate) fn apply_commands(
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

    /// Read-only predicate used by the host to choose between its view-cone
    /// selection and an admitted `EzekielInstakill` frame action.
    pub fn can_ezekiel_instakill(&self, id: EntityId) -> bool {
        self.inner.can_ezekiel_instakill(id)
    }

    // ── Setup / lifecycle ──────────────────────────────────────────

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
    pub(crate) fn run_console_command(
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
    pub(crate) fn run_cheat_string(
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

    /// Invoke a script native with an explicit transient `ThisActor` receiver.
    /// Runtime callers reach this only through [`Self::advance_frame`].
    pub(crate) fn call_external_native_with_this(
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

    /// Insert a fully-formed entity into a test engine. Input-resolution
    /// tests need live entities to click on; the blank `new_for_test`
    /// level has none and the production spawn path requires proto data.
    #[cfg(feature = "test-helpers")]
    #[doc(hidden)]
    pub fn test_add_entity(&mut self, entity: crate::element::Entity) -> EntityId {
        self.inner.add_entity(entity)
    }

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
    use crate::engine::SimCommand;

    fn frame_api_fixture() -> (Engine, LevelAssets) {
        let mut assets = LevelAssets::new();
        let mut sim_config = SimConfig::default();
        sim_config.script_enabled = false;
        sim_config.ignore_default_loose = true;
        let engine = Engine::new_for_test_with_simulation(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            0xF4A6_E001,
            sim_config,
        )
        .expect("construct frame API fixture");
        (engine, assets)
    }

    #[test]
    fn frame_api_matches_legacy_command_then_hourglass_boundary() {
        let (engine, assets) = frame_api_fixture();
        let mut legacy = engine.clone();
        let mut framed = engine;
        let commands = vec![PlayerInput::host(
            PlayerCommand::SetMenToBlazonConversionMode { on: true },
        )];

        let mut legacy_display = super::super::HostDisplayState::default();
        let mut legacy_input = InputState::default();
        let mut legacy_dev = DevState::default();
        legacy.apply_commands(&mut legacy_display, &mut legacy_input, &assets, &commands);
        let legacy_events = legacy.perform_hourglass(&mut legacy_display, &assets, &mut legacy_dev);
        let legacy_hash = crate::replay::state_hash(&legacy);

        let mut framed_display = super::super::HostDisplayState::default();
        let mut framed_input = InputState::default();
        let mut framed_dev = DevState::default();
        let output = framed
            .advance_frame(
                &mut framed_display,
                &mut framed_input,
                &assets,
                &mut framed_dev,
                SimulationFrameInput::from_player_inputs(commands),
            )
            .expect("advance frame");

        assert_eq!(output.frame_before, 0);
        assert_eq!(output.frame_after, 1);
        assert_eq!(output.state_hash, legacy_hash);
        assert_eq!(crate::replay::state_hash(&framed), legacy_hash);
        assert_eq!(
            serde_json::to_value(output.events.side_effects()).expect("serialize frame events"),
            serde_json::to_value(&legacy_events).expect("serialize legacy side effects"),
        );
        assert_eq!(
            output.events.side_effects().pending_minimap_position,
            legacy_events.pending_minimap_position,
            "this host-local effect is serde-skipped and must be compared explicitly"
        );
        assert_eq!(
            serde_json::to_value(&framed_display).expect("serialize framed display"),
            serde_json::to_value(&legacy_display).expect("serialize legacy display"),
        );
        assert!(framed.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn frame_api_applies_sound_external_fact_at_pre_hourglass_boundary() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut framed, assets) = frame_api_fixture();
        let profile_id = 0x4651_0000;
        framed
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id,
                exclamation_id: 62,
                variant: -1,
            });
        let resolution = ResolvedExclamation {
            actor_id: 191,
            identifier: profile_id | 62,
            exclamation_id: 62,
            duration_frames: 24,
        };

        let mut display = super::super::HostDisplayState::default();
        let mut input = InputState::default();
        let mut dev = DevState::default();
        framed
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut dev,
                SimulationFrameInput::new(vec![SimCommand::from(PlayerCommand::Noop)])
                    .with_external_facts(
                        ExternalFacts::default()
                            .with_sound_boundary(SoundBoundary::live(vec![resolution])),
                    ),
            )
            .expect("advance frame with sound fact");

        assert!(framed.sound_sim().resolved_exclamations.is_empty());
        assert_eq!(framed.sound_sim().playing_exclamations.len(), 1);
        assert_eq!(framed.sound_sim().playing_exclamations[0].actor_id, 191);
        assert_eq!(
            framed.sound_sim().playing_exclamations[0].finish_frame,
            24,
            "the fact is resolved at the frame-0 boundary before the hourglass increments the clock"
        );
    }

    #[test]
    fn rejected_live_sound_boundary_is_atomic() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut engine, assets) = frame_api_fixture();
        engine
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id: 0x4651_0000,
                exclamation_id: 62,
                variant: -1,
            });
        let invalid_resolution = ResolvedExclamation {
            actor_id: 192,
            identifier: 0x4651_003f,
            exclamation_id: 63,
            duration_frames: 24,
        };
        let engine_hash_before = crate::replay::state_hash(&engine);
        let mut display = super::super::HostDisplayState::default();
        let display_before =
            serde_json::to_value(&display).expect("serialize display before rejected boundary");
        let mut input = InputState::default();
        let mut dev = DevState::default();

        let error = engine
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut dev,
                SimulationFrameInput::new(vec![SimCommand::from(
                    PlayerCommand::SetMenToBlazonConversionMode { on: true },
                )])
                .with_external_facts(
                    ExternalFacts::default()
                        .with_sound_boundary(SoundBoundary::live(vec![invalid_resolution])),
                ),
            )
            .expect_err("a live sound resolution must match the pending FIFO");

        assert!(matches!(
            error,
            FrameAdvanceError::SoundBoundaryRejected {
                policy: SoundBoundaryPolicy::Live,
                ..
            }
        ));
        assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
        assert_eq!(
            serde_json::to_value(&display).expect("serialize display after rejected boundary"),
            display_before,
        );
        assert_eq!(engine.frame_counter(), 0);
        assert!(!engine.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn rejected_external_fact_prevents_command_and_hourglass() {
        use crate::element::Command;
        use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

        let (mut engine, assets) = frame_api_fixture();
        engine.set_external_director_completion_replay(true);

        // Launch a real camera command. The first completion therefore mutates
        // the Engine before the second, invalid completion is rejected.
        let mut camera = SequenceElement::new_generic(1, Command::CameraGoto, None);
        camera.set_property(
            Field::CameraPoint,
            FieldValue::GeoPoint2D { x: 100.0, y: 100.0 },
        );
        camera.set_property(Field::CameraSpeed, FieldValue::Integer(0));
        let mut sequence = Sequence::new();
        sequence.append_element(camera);
        let sequence_id = engine
            .inner
            .orders
            .sequence_manager
            .launch_sequence(sequence);
        engine
            .inner
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        let mut display = super::super::HostDisplayState::default();
        engine.inner.feedback.cutscene_camera.sequence_element =
            Some(crate::sequence::SequenceElementRef::new(sequence_id, 0));
        assert!(
            engine
                .inner
                .feedback
                .cutscene_camera
                .sequence_element
                .is_some(),
            "fixture must have an active CameraGoto"
        );

        let engine_hash_before = crate::replay::state_hash(&engine);
        let display_before =
            serde_json::to_value(&display).expect("serialize display before rejected frame");
        let mut accepted_engine = engine.clone();
        let mut accepted_display = display.clone();
        accepted_engine
            .apply_external_director_completion(
                DirectorCompletion::CameraGoto,
                &mut accepted_display,
                &assets,
            )
            .expect("the first director fact must be independently valid");
        assert_ne!(
            crate::replay::state_hash(&accepted_engine),
            engine_hash_before,
            "the accepted prefix must mutate the staged engine"
        );
        let mut input = InputState::default();
        input.right_mouse_down = true;
        let mut dev = DevState::default();
        dev.projectile_cheat_rain = 7;

        let error = engine
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut dev,
                SimulationFrameInput::new(vec![
                    SimCommand::from(PlayerCommand::SetMenToBlazonConversionMode { on: true }),
                    SimCommand::from(PlayerCommand::MouseRightUp),
                ])
                .with_external_facts(
                    ExternalFacts::default().with_director_completions(vec![
                        DirectorCompletion::CameraGoto,
                        DirectorCompletion::CameraGoto,
                    ]),
                ),
            )
            .expect_err("the second completion has no active camera command");

        assert!(matches!(
            error,
            FrameAdvanceError::DirectorCompletionRejected {
                index: 1,
                completion: DirectorCompletion::CameraGoto,
                ..
            }
        ));
        assert_eq!(crate::replay::state_hash(&engine), engine_hash_before);
        assert_eq!(engine.frame_counter(), 0);
        assert!(!engine.is_men_to_blazon_conversion_mode());
        assert_eq!(
            serde_json::to_value(&display).expect("serialize display after rejected frame"),
            display_before,
        );
        assert!(input.right_mouse_down, "commands must not have run");
        assert_eq!(
            dev.projectile_cheat_rain, 7,
            "the hourglass must not have run"
        );
    }

    #[test]
    fn external_facts_are_part_of_the_authoritative_frame_journal() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let (mut initial, assets) = frame_api_fixture();
        let profile_id = 0x4651_0000;
        initial
            .inner
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id,
                exclamation_id: 62,
                variant: -1,
            });
        let mut complete_journal = initial.clone();
        let mut command_only_journal = initial;
        let command = SimCommand::from(PlayerCommand::Noop);
        let resolution = ResolvedExclamation {
            actor_id: 191,
            identifier: profile_id | 62,
            exclamation_id: 62,
            duration_frames: 24,
        };

        let mut complete_display = super::super::HostDisplayState::default();
        let mut complete_input = InputState::default();
        let mut complete_dev = DevState::default();
        let complete_output = complete_journal
            .advance_frame(
                &mut complete_display,
                &mut complete_input,
                &assets,
                &mut complete_dev,
                SimulationFrameInput::new(vec![command.clone()]).with_external_facts(
                    ExternalFacts::default()
                        .with_sound_boundary(SoundBoundary::live(vec![resolution])),
                ),
            )
            .expect("advance complete frame journal");

        let mut command_only_display = super::super::HostDisplayState::default();
        let mut command_only_input = InputState::default();
        let mut command_only_dev = DevState::default();
        let command_only_output = command_only_journal
            .advance_frame(
                &mut command_only_display,
                &mut command_only_input,
                &assets,
                &mut command_only_dev,
                SimulationFrameInput::new(vec![command]),
            )
            .expect("advance command-only frame journal");

        assert_ne!(
            complete_output.state_hash, command_only_output.state_hash,
            "replaying commands without the recorded host sound fact must not be treated as equivalent"
        );
        assert_eq!(complete_journal.sound_sim().playing_exclamations.len(), 1);
        assert!(complete_journal.sound_sim().pending_exclamations.is_empty());
        assert!(
            command_only_journal
                .sound_sim()
                .playing_exclamations
                .is_empty()
        );
        assert_eq!(
            command_only_journal.sound_sim().pending_exclamations.len(),
            1
        );
    }

    #[test]
    fn closed_body_gate_is_not_a_paused_presentation_boundary() {
        let (mut engine, assets) = frame_api_fixture();
        let mut display = super::super::HostDisplayState::default();
        let mut input = InputState::default();
        let mut dev = DevState::default();

        let output = engine
            .advance_frame(
                &mut display,
                &mut input,
                &assets,
                &mut dev,
                SimulationFrameInput::default().with_simulation_body_allowed(false),
            )
            .expect("advance with only the actor/world body gated");

        assert_eq!(output.frame_before, 0);
        assert_eq!(output.frame_after, 1);
        assert_eq!(engine.frame_counter(), 1);
    }

    #[test]
    fn parity_engine_state_preserves_next_original_creation_order() {
        let mut inner = EngineInner::new();
        inner.world.next_original_creation_order = 417;
        inner.control.chorus_timer = 23;
        inner.script_domains.mission_ui.force_check = true;
        inner
            .script_domains
            .mission_ui
            .men_to_blazon_conversion_mode = true;
        let state = Engine { inner }.parity_engine_state();

        assert_eq!(state.next_creation_order, 417);
        assert_eq!(state.chorus_timer, 23);
        assert!(state.force_check);
        assert!(state.men_to_blazon_conversion);
    }

    #[test]
    fn parity_game_ui_state_preserves_serialized_latches() {
        let mut inner = EngineInner::new();
        let ui = &mut inner.script_domains.mission_ui;
        ui.campaign_map = true;
        ui.campaign_map_displayed = true;
        ui.game_post_initialized = true;
        ui.start_mission_disabled_temp = true;
        ui.quit_mission_disabled_temp = false;
        ui.start_mission_enabled = true;
        ui.quit_mission_enabled = false;

        assert_eq!(
            Engine { inner }.parity_game_ui_state(),
            serde_json::json!({
                "campaign_map": true,
                "campaign_map_displayed": true,
                "post_initialized": true,
                "start_mission_disabled_temp": true,
                "quit_mission_disabled_temp": false,
                "start_mission_enabled": true,
                "quit_mission_enabled": false,
            })
        );
    }

    #[test]
    fn parity_messenger_controller_is_independent_of_camera_locker() {
        let mut inner = EngineInner::new();
        inner.players.view_locked = true;
        inner.players.seats[0].locker_active = false;
        inner.players.seats[0].selected_action = crate::profiles::Action::Bow;

        let engine = Engine { inner };
        assert_eq!(
            engine.parity_messenger_controller_state(),
            serde_json::json!({ "view_locked": true, "selected_action": 1 })
        );
        assert!(!engine.locker_active());
        assert!(engine.view_locked());
    }

    #[test]
    fn parity_shield_controller_preserves_global_protocol_state() {
        let mut inner = EngineInner::new();
        inner.world.shield.is_protected = false;
        inner.world.shield.protected_pc = Some(EntityId::new(7, crate::element::EntityIdKind::Pc));
        inner.world.shield.danger_point = crate::coordinates::WorldPoint3D {
            x: 1.25,
            y: -2.5,
            z: 3.75,
        };

        assert_eq!(
            Engine { inner }.parity_shield_controller_state(),
            serde_json::json!({
                "is_protected": false,
                "protected_pc": { "kind": "pc", "index": 7 },
                "danger_point": {
                    "x": { "bits": 1.25_f32.to_bits() },
                    "y": { "bits": (-2.5_f32).to_bits() },
                    "z": { "bits": 3.75_f32.to_bits() },
                },
            })
        );
    }

    #[test]
    fn parity_sound_sources_preserves_sparse_slots_and_authoritative_fields() {
        let mut inner = EngineInner::new();
        inner.feedback.sound_sim.sources.sources_push_none();
        let mut source = crate::sound_source::SoundSource::new();
        source.source_kind = crate::sound_source::SoundSourceKind::Delayed;
        source.id = 73;
        source.inner_distance = 12;
        source.outer_distance = 34;
        source.noise_covering_distance = 56;
        source.inner_volume = 78;
        source.outer_volume = 9;
        source
            .shape
            .push(crate::coordinates::MapPoint::new(1.5, -2.0));
        source.altitude = crate::sound_geometry::SoundSourceAltitude::Top;
        source.min_delay = 4;
        source.max_delay = 18;
        source.delay_stepping = 5;
        source.timer = 11;
        source.active = true;
        inner.feedback.sound_sim.sources.sources_push_some(source);
        let engine = Engine { inner };

        let state = engine.parity_sound_sources_state();
        assert!(state[0].is_null());
        assert_eq!(state[1]["kind"], 2);
        assert_eq!(state[1]["id"], 73);
        assert_eq!(state[1]["noise_covering_distance"], 56);
        assert_eq!(state[1]["shape"][0]["x"]["bits"], 1.5f32.to_bits());
        assert_eq!(state[1]["altitude"], 2);
        assert_eq!(state[1]["timer"], 11);
        assert_eq!(state[1]["active"], true);
    }

    #[test]
    fn parity_sound_completion_frontier_preserves_pending_order() {
        let mut inner = EngineInner::new();
        inner
            .feedback
            .sound_sim
            .sources
            .sources_push_some(crate::sound_source::SoundSource::new());
        inner
            .feedback
            .sound_sim
            .sources
            .sources_push_some(crate::sound_source::SoundSource::new());
        inner
            .feedback
            .sound_sim
            .playing_sources
            .push(crate::sound::PlayingSource {
                source_index: 1,
                finish_frame: 73,
            });
        inner
            .feedback
            .sound_sim
            .playing_sources
            .push(crate::sound::PlayingSource {
                source_index: 0,
                finish_frame: 91,
            });

        let state = Engine { inner }.parity_sound_completion_frontier_state();
        assert_eq!(state[0]["source_index"], 1);
        assert_eq!(state[0]["finish_frame"], 73);
        assert_eq!(state[1]["source_index"], 0);
        assert_eq!(state[1]["finish_frame"], 91);
    }

    #[test]
    fn parity_ai_global_preserves_ordered_statuses_reservations_and_alerts() {
        let mut inner = EngineInner::new();
        inner.ai.global.stupid_soldiers_cheat = true;
        inner.ai.global.green_alert_soldiers = 3;
        inner.ai.global.yellow_alert_soldiers = 4;
        inner.ai.global.red_alert_soldiers = 5;
        inner.ai.global.overall_alert_status = crate::ai::AlertLevel::Yellow;
        inner.ai.global.overall_villain_alert_status = crate::ai::AlertLevel::Red;
        inner.ai.global.saved_random_seed = -73;
        inner.ai.global.current_speech_variant = 2;
        inner
            .ai
            .global
            .forbidden_remarks
            .push(crate::ai::ForbiddenRemark {
                remark: crate::ai::Remark::Warcry,
                flags: crate::ai::RemarkTargetFlags::THIS_GUY.bits(),
                speech_id: 91,
                guy_index: 47,
                bad_guy: true,
                forbidden_till_frame: 1234,
            });
        let mut seek = crate::ai::SeekPoint::from_position(
            &crate::sim_rng::SimulationContext::with_seed(1),
            crate::ai::Position::default(),
        );
        seek.frame_when_full_interest = 99;
        seek.last_calculated_interest = 41;
        seek.locked = true;
        inner.ai.global.seek_points.push(seek);
        inner
            .ai
            .global
            .archery_sectors
            .push(crate::ai::SectorArchery {
                points: vec![crate::ai::PointArchery {
                    position: crate::ai::Position::default(),
                    direction: 7,
                    is_shooting_point: true,
                    sector_index: crate::sector::SectorNumber(2),
                    owner: None,
                }],
                polygon: Vec::new(),
                layer: 0,
                index_first_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
                index_last_shooting_point: Some(crate::sector::ArcheryPointIdx(0)),
                num_shooting_points: 1,
                num_owners: 0,
            });
        let engine = Engine { inner };

        let state = engine.parity_ai_global_state();
        assert_eq!(state["stupid_soldiers_cheat"], true);
        assert_eq!(state["seek_points"][0]["frame_when_full_interest"], 99);
        assert_eq!(state["seek_points"][0]["last_calculated_interest"], 41);
        assert_eq!(state["seek_points"][0]["locked"], true);
        assert_eq!(state["archery_sectors"][0]["num_owners"], 0);
        assert!(state["archery_sectors"][0]["point_owners"][0].is_null());
        assert_eq!(state["overall_alert_status"], 1);
        assert_eq!(state["overall_villain_alert_status"], 2);
        assert_eq!(state["saved_random_seed"], -73);
        assert_eq!(state["forbidden_remarks"][0]["remark"], 9);
        assert_eq!(state["forbidden_remarks"][0]["flags"], 8);
        assert_eq!(state["forbidden_remarks"][0]["speech_id"], 91);
        assert_eq!(state["forbidden_remarks"][0]["guy_index"], 47);
        assert_eq!(state["forbidden_remarks"][0]["bad_guy"], true);
        assert_eq!(state["forbidden_remarks"][0]["forbidden_till_frame"], 1234);
        assert_eq!(state["current_speech_variant"], 2);
    }

    #[test]
    fn parity_pc_registry_preserves_original_order_not_portrait_order() {
        let mut inner = EngineInner::new();
        let new_pc = || {
            crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData::default(),
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                pc: crate::element::PcData::default(),
            })
        };
        let first = inner.add_entity(new_pc());
        let second = inner.add_entity(new_pc());
        inner.world.pc_ids = vec![first, second];
        inner.world.original_pc_registry_ids = vec![second, first];

        let state = Engine { inner }.parity_pc_registry_state();
        assert_eq!(state[0]["kind"], "pc");
        assert_eq!(state[0]["index"], second.index());
        assert_eq!(state[1]["index"], first.index());
    }

    #[test]
    fn parity_runtime_roots_preserves_mission_stat_and_empty_reference_roots() {
        struct MenuText;
        impl crate::sherwood_stat::MenuTextLookup for MenuText {
            fn get(&self, id: usize) -> String {
                format!("menu-{id}")
            }
        }

        let mut inner = EngineInner::new();
        inner.players.user_locked = true;
        inner.mission_domain.mission_stat.collected_money = 73;
        inner.mission_domain.mission_stat.added_score = 91;
        inner
            .mission_domain
            .mission_stat
            .pc_names
            .push(crate::mission_stat::PcStatName::new(
                "fallback".into(),
                Some(crate::pc_status::SpecialPeasantName::B),
            ));
        let engine = Engine { inner };

        let state = engine.parity_engine_runtime_roots_state(&MenuText);
        assert_eq!(state["timer_elements"].as_array().unwrap().len(), 0);
        assert!(state["camera_sequence"].is_null());
        assert!(state["dead_pc"].is_null());
        assert_eq!(state["mission_stat"]["collected_money"], 73);
        assert_eq!(state["mission_stat"]["added_score"], 91);
        assert_eq!(state["mission_stat"]["pc_names"][0], "menu-251");
        assert_eq!(state["user_locked"], true);
        assert_eq!(
            state["selection_before_user_lock"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert!(state["follow_element"].is_null());
    }

    #[test]
    fn parity_world_interactables_preserves_dynamic_patch_and_door_fields() {
        let mut inner = EngineInner::new();
        let mut patch = crate::patch::Patch::default();
        patch.active = true;
        patch.locked = true;
        patch.applied = true;
        patch.in_transition = true;
        inner.script_domains.interactables.patches.push(patch);
        let mut door = crate::gate::Door::default();
        door.active = false;
        door.locked_pc = true;
        door.locked_npc_villain = true;
        door.unlockable = true;
        door.locked_pc_after_patch = true;
        door.locked_npc_civilian_after_patch = true;
        door.unlockable_after_patch = true;
        door.special_authorisation_pc = true;
        door.authorised_pc_direct = 0x12;
        door.authorised_pc_indirect = 0x34;
        inner.script_domains.interactables.doors.push(door);
        let engine = Engine { inner };

        let state = engine.parity_world_interactables_state(&LevelAssets::new());
        assert_eq!(state["patches"][0]["active"], true);
        assert_eq!(state["patches"][0]["locked"], true);
        assert_eq!(state["patches"][0]["applied"], true);
        assert_eq!(state["patches"][0]["in_transition"], true);
        assert_eq!(
            state["patches"][0]["occupants"].as_array().unwrap().len(),
            0
        );
        assert_eq!(state["doors"][0]["kind"], "door");
        assert_eq!(state["doors"][0]["active"], false);
        assert_eq!(state["doors"][0]["locked_pc"], true);
        assert_eq!(state["doors"][0]["locked_npc_villain"], true);
        assert_eq!(state["doors"][0]["unlockable"], true);
        assert_eq!(state["doors"][0]["locked_pc_after_patch"], true);
        assert_eq!(state["doors"][0]["locked_npc_civilian_after_patch"], true);
        assert_eq!(state["doors"][0]["unlockable_after_patch"], true);
        assert_eq!(state["doors"][0]["special_authorisation_pc"], true);
        assert_eq!(state["doors"][0]["authorised_pc_direct"], 0x12);
        assert_eq!(state["doors"][0]["authorised_pc_indirect"], 0x34);
        assert_eq!(state["sector_doors"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn parity_world_interactables_preserves_lift_runtime_state() {
        let mut inner = EngineInner::new();
        let sector_number = crate::sector::SectorNumber::new(47);
        let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
        level.sector_number_map.insert(sector_number, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        inner.world.fast_grid_mut().lift_state.insert(
            0,
            crate::fast_find_grid::LiftRuntimeState {
                occupants_pc: 2,
                occupants: 3,
                occupied_upwards: true,
                occupied_downwards: false,
                wait_time: 71,
            },
        );

        let state = Engine { inner }.parity_world_interactables_state(&LevelAssets::new());
        assert_eq!(state["lifts"][0]["sector"], 47);
        assert_eq!(state["lifts"][0]["occupants_pc"], 2);
        assert_eq!(state["lifts"][0]["occupants"], 3);
        assert_eq!(state["lifts"][0]["occupied_upwards"], true);
        assert_eq!(state["lifts"][0]["occupied_downwards"], false);
        assert_eq!(state["lifts"][0]["wait_time"], 71);
    }

    #[test]
    fn parity_world_interactables_preserves_ordered_building_and_zone_state() {
        let mut inner = EngineInner::new();
        let new_pc = || {
            crate::element::Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData::default(),
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                pc: crate::element::PcData::default(),
            })
        };
        let first = inner.add_entity(new_pc());
        let second = inner.add_entity(new_pc());
        inner.script_domains.buildings.occupants.push(vec![
            crate::natives::ScriptHandleCodec::actor_handle(second),
            crate::natives::ScriptHandleCodec::actor_handle(first),
        ]);
        inner.script_domains.buildings.arrow_reserves.push(true);

        inner
            .script_domains
            .zones
            .scripts
            .push(crate::sector::ScriptSectorData {
                sector_index: crate::fast_find_grid::SectorIndex::new(0),
                transformed_to_apex: true,
                max_throwing_apex_height: 12.5,
                occupant_indices: vec![first, second],
                ..Default::default()
            });
        let level = std::sync::Arc::make_mut(&mut inner.world.fast_grid_mut().level);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::SCRIPT,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(47),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        inner
            .world
            .fast_grid_mut()
            .or_sector_type_overlay(0, crate::sector::SectorType::APEX);
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.scripts.zone_grid_indices).push(0);

        let state = Engine { inner }.parity_world_interactables_state(&assets);
        assert_eq!(
            state["buildings"][0]["occupants"][0]["index"],
            second.index()
        );
        assert_eq!(
            state["buildings"][0]["occupants"][1]["index"],
            first.index()
        );
        assert_eq!(state["buildings"][0]["arrow_reserve"], true);
        assert_eq!(
            state["script_zones"][0]["occupants"][0]["index"],
            first.index()
        );
        assert_eq!(
            state["script_zones"][0]["occupants"][1]["index"],
            second.index()
        );
        assert_eq!(state["script_zones"][0]["transformed_to_apex"], true);
        assert_eq!(
            state["script_zones"][0]["max_apex_height"]["bits"],
            12.5f32.to_bits()
        );
    }

    #[test]
    fn parity_repulsive_points_preserves_serialized_fields_order_and_next_id() {
        let mut inner = EngineInner::new();
        inner.world.original_repulsive_point_counter = 42;
        let mut first = crate::ai::RepulsivePoint::new(
            17,
            crate::ai::Position {
                x: 1.25,
                y: -2.5,
                sector: None,
                level: 3,
            },
            4.0,
            5.0,
            1 | 4 | 8,
        );
        first.concave = true;
        first.limit_left = crate::coordinates::MapVec::new(6.0, 7.0);
        first.limit_right = crate::coordinates::MapVec::new(8.0, 9.0);
        inner.ai.global.repulsive_points.push(first);
        inner
            .ai
            .global
            .repulsive_points
            .push(crate::ai::RepulsivePoint::new(
                18,
                crate::ai::Position {
                    level: 5,
                    ..Default::default()
                },
                10.0,
                11.0,
                2,
            ));
        let engine = Engine { inner };

        let state = engine.parity_repulsive_points_state();
        assert_eq!(state["next_id"], 42);
        assert_eq!(state["points"][0]["id"], 17);
        assert_eq!(state["points"][1]["id"], 18);
        assert_eq!(
            state["points"][0]["position"]["x"]["bits"],
            1.25f32.to_bits()
        );
        assert_eq!(
            state["points"][0]["position"]["y"]["bits"],
            (-2.5f32).to_bits()
        );
        assert_eq!(state["points"][0]["concave"], true);
        assert_eq!(
            state["points"][0]["limit_left"]["x"]["bits"],
            6.0f32.to_bits()
        );
        assert_eq!(
            state["points"][0]["limit_right"]["y"]["bits"],
            9.0f32.to_bits()
        );
        assert_eq!(state["points"][0]["radius"]["bits"], 4.0f32.to_bits());
        assert_eq!(
            state["points"][0]["action_radius"]["bits"],
            9.0f32.to_bits()
        );
        assert_eq!(state["points"][0]["affects_pcs"], true);
        assert_eq!(state["points"][0]["affects_soldiers"], false);
        assert_eq!(state["points"][0]["affects_civilians"], true);
        assert_eq!(state["points"][0]["affects_animals"], true);
        assert_eq!(state["points"][0]["layer"], 3);
    }

    #[test]
    fn parity_titbits_preserves_serialized_manager_and_live_entry_fields() {
        let mut inner = EngineInner::new();
        let id = inner.feedback.titbit_manager.add_titbit(
            crate::coordinates::WorldPoint3D::new(1.5, -2.0, 3.25),
            4,
            crate::titbit::TitbitKind::DangerPoint,
            crate::titbit::ElementHandle::INVALID,
            7,
            crate::titbit::ElementHandle::INVALID,
            false,
            crate::titbit::INVALID_ID,
            true,
            None,
            None,
        );
        let titbit = &mut inner.feedback.titbit_manager.titbits_mut()[0];
        titbit.sprite_row = 8;
        titbit.sprite_frame = 9;
        titbit.frame_count = 10;
        titbit.display_order = 11.5;
        titbit.blinking = true;
        let engine = Engine { inner };

        let state = engine.parity_titbit_manager_state();
        assert_eq!(state["current_id"], 1);
        assert_eq!(state["titbits"][0]["kind"], 10);
        assert_eq!(state["titbits"][0]["phase"], 7);
        assert_eq!(state["titbits"][0]["sprite_row"], 8);
        assert_eq!(state["titbits"][0]["sprite_frame"], 9);
        assert_eq!(state["titbits"][0]["frame_count"], 10);
        assert_eq!(
            state["titbits"][0]["display_order"]["bits"],
            11.5f32.to_bits()
        );
        assert_eq!(state["titbits"][0]["layer"], 4);
        assert_eq!(state["titbits"][0]["blinking"], true);
        assert_eq!(state["titbits"][0]["id"], id);
        assert!(state["titbits"][0]["element_supplier"].is_null());
        assert!(state["titbits"][0]["element_manager"].is_null());
        assert_eq!(
            state["titbits"][0]["position"]["x"]["bits"],
            1.5f32.to_bits()
        );
    }

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
        malformed_inner.world.fast_grid_mut().line_active.push(true);
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
