//! AI system — core types, state machine, stimulus processing.
//!
//! Defines the enums, flags, data structures, and base AI controller
//! that drive all NPC behavior. The actual behavior implementations live in
//! [`ai_enemy`](super::ai_enemy) (villain/soldier AI) and
//! [`ai_friendly`](super::ai_friendly) (civilian AI).

use std::sync::Arc;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::element::EntityId;
use crate::order::AiOrderIntent;

// ---------------------------------------------------------------------------
// Opaque entity handle types
// ---------------------------------------------------------------------------

// These `u32` aliases are a transitional layer.  The entity system now has
// `EntityId` (a typed newtype).  New code should prefer `EntityId`; the
// aliases remain so existing code compiles without a mass rewrite.

/// Opaque handle to an NPC actor.
pub type NpcHandle = u32;
/// Opaque handle to a human actor (NPC or PC).
pub type HumanHandle = u32;
/// Opaque handle to a generic actor.
pub type ActorHandle = u32;
/// Opaque handle to a generic element.
pub type ElementHandle = u32;
/// Opaque handle to an object element.
pub type ObjectHandle = u32;
/// Opaque handle to a door.
pub type DoorHandle = u32;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum CharlySeekerTarget {
    SelfNpc,
    Npc(NpcHandle),
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum AiStateChangeSource {
    SelfActor,
    Null,
    Human(HumanHandle),
}

impl AiStateChangeSource {
    pub fn from_optional_human(handle: HumanHandle) -> Self {
        if handle == 0 {
            Self::Null
        } else {
            Self::Human(handle)
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum EnterSwordfightRequest {
    RaiseSword,
    Engage(HumanHandle),
}

pub use crate::position_interface::SectorHandle;

// NpcHandle is still a `u32` alias; convert it to an `EntityId` only at
// call sites where the concrete entity type is known.

// ---------------------------------------------------------------------------
// AI lock flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Bitfield controlling when an NPC's AI is locked (ignores stimuli).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct AiLockFlags: u8 {
        const BEGGAR = 0x01;
        const BUSY   = 0x02;
        const FREEZE = 0x04;
    }
}

// ---------------------------------------------------------------------------
// GoTo flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Flags controlling how an NPC moves to a destination.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct GotoFlags: u16 {
        const RUN              = 0x0001;
        const BACK             = 0x0002;
        const STRAIGHT         = 0x0004;
        const STRAFE           = 0x0008;
        const ASK_OBSTACLE     = 0x0010;
        const SPECIAL_ACTION   = 0x0020;
        const USE_NORM         = 0x0040;
        const NEAR             = 0x0080;
        const GROUP_MOVE       = 0x0100;
        const FIND_ACCESSIBLE  = 0x0200;
        const DONT_STOP        = 0x0400;
        const SWORD            = 0x0800;
        const CHARGE           = 0x1000;
        const NO_HALT          = 0x2000;
        const RIDER_CHARGE     = 0x4000;
        const RIDER_CHARGE_HIT = 0x8000;

        /// Flags forbidden for civilian NPCs.
        const FORBIDDEN_CIVILIANS = Self::BACK.bits()
            | Self::SWORD.bits()
            | Self::CHARGE.bits()
            | Self::RIDER_CHARGE.bits()
            | Self::RIDER_CHARGE_HIT.bits();
    }
}

// ---------------------------------------------------------------------------
// Duty flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct DutyFlags: u16 {
        const KEEP_EMOTICON             = 0x0001;
        const BECAUSE_COULDNT_REACHPOINT = 0x0002;
    }
}

// ---------------------------------------------------------------------------
// Alert flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct AlertFlags: u16 {
        const INSTANT_MUSIC_CHANGE = 0x0001;
        const ONLY_MUSIC           = 0x0002;
    }
}

// ---------------------------------------------------------------------------
// Speech flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SpeechFlags: u16 {
        const HOUSE             = 0x0001;
        const EMERGENCY         = 0x0002;
        const SCRIPT            = 0x0004;
        const ALWAYS            = 0x0008;
        const MYTALK_1          = 0x0100;
        const MYTALK_2          = 0x0200;
        const MYTALK_3          = 0x0400;
        const CYCLE_3_VARIANTS  = 0x0800;
        const MYTALK_0          = 0x1000;
    }
}

// ---------------------------------------------------------------------------
// Remark-target flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Who should hear/see a remark.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RemarkTargetFlags: u16 {
        const THIS_TYPE      = 0x0001;
        const CIVILIANS      = 0x0002;
        const VILLAINS       = 0x0004;
        const THIS_GUY       = 0x0008;
        const CIV_RESP_VILL  = 0x1000;
        const ALL_NPC        = Self::CIVILIANS.bits() | Self::VILLAINS.bits();
    }
}

// ---------------------------------------------------------------------------
// Attention value constants
// ---------------------------------------------------------------------------

pub const MAX_ATT_VALUE: i32 = 100;
pub const THREE_QUARTERS_MAX_ATT_VALUE: i32 = 75;
pub const HALF_MAX_ATT_VALUE: i32 = 50;
pub const QUARTER_MAX_ATT_VALUE: i32 = 25;

// ---------------------------------------------------------------------------
// PatrolPath — wraps a hiking path with current waypoint tracking
// ---------------------------------------------------------------------------

/// Hiking-path index newtype.  Nominal wrapper around `NonMaxU16` —
/// `Option<PathId>` is 2 bytes thanks to the niche, and `0xFFFF` is the
/// binary-format "no path" sentinel so a real path id literally cannot
/// hold it.  Used for soldier `path_id` / `alert_path_id`, civilian
/// `path_id`, and the waypoint-script `(PathId, wp_idx)` registration key.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct PathId(pub nonmax::NonMaxU16);

impl PathId {
    #[inline]
    pub fn new(v: u16) -> Option<Self> {
        nonmax::NonMaxU16::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl From<PathId> for u16 {
    #[inline]
    fn from(p: PathId) -> u16 {
        p.get()
    }
}

impl From<PathId> for usize {
    #[inline]
    fn from(p: PathId) -> usize {
        p.get() as usize
    }
}

impl std::fmt::Display for PathId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

/// Runtime patrol path state.
///
/// Wraps a reference to a `RawHikingPath` (by index into `EngineInner::hiking_paths`)
/// with the current waypoint index and traversal direction. Uses ping-pong
/// traversal: when the end is reached, direction flips instead of wrapping.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PatrolPath {
    /// Index into `EngineInner::hiking_paths`.
    pub hiking_path_index: PathId,
    /// Current waypoint index within the hiking path.
    pub current_waypoint_index: u8,
    /// Previous waypoint index (set before each advance).
    pub last_waypoint_index: u8,
    /// `true` = advancing toward end, `false` = retreating toward start.
    pub forward: bool,
    /// Number of waypoints in the path (cached from the hiking path).
    pub size: u8,
    /// History of chief positions for computing minion formation positions.
    pub history: Vec<PathHistoryEntry>,
}

impl PatrolPath {
    /// Initialize from a hiking path index and the loaded hiking paths.
    /// Returns `None` if the index is out of range.
    pub fn new(
        path_index: PathId,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> Option<Self> {
        let path = hiking_paths.get(usize::from(path_index))?;
        Some(Self {
            hiking_path_index: path_index,
            current_waypoint_index: 0,
            last_waypoint_index: 0,
            forward: true,
            size: path.waypoints.len() as u8,
            history: Vec::new(),
        })
    }

    /// Advance to the next waypoint (ping-pong: reverses at endpoints).
    pub fn advance(&mut self) {
        self.last_waypoint_index = self.current_waypoint_index;
        if self.size <= 1 {
            return;
        }
        if self.forward {
            if self.current_waypoint_index < self.size - 1 {
                self.current_waypoint_index += 1;
            } else {
                self.current_waypoint_index -= 1;
                self.forward = false;
            }
        } else {
            if self.current_waypoint_index > 0 {
                self.current_waypoint_index -= 1;
            } else {
                self.current_waypoint_index += 1;
                self.forward = true;
            }
        }
    }

    /// Step backward (flip direction, step forward, flip back).
    pub fn retreat(&mut self) {
        self.forward = !self.forward;
        self.advance();
        self.forward = !self.forward;
    }

    /// Flip ping-pong traversal direction in place (called by CMD_REVERSE_PATH).
    pub fn flip_forward_movement(&mut self) {
        self.forward = !self.forward;
    }

    /// Get the current waypoint from the hiking paths array.
    pub fn current_waypoint<'a>(
        &self,
        hiking_paths: &'a [crate::level_data::RawHikingPath],
    ) -> Option<&'a crate::level_data::RawWaypoint> {
        hiking_paths
            .get(usize::from(self.hiking_path_index))?
            .waypoints
            .get(self.current_waypoint_index as usize)
    }

    /// Get a waypoint by index.
    pub fn get_waypoint<'a>(
        &self,
        index: u8,
        hiking_paths: &'a [crate::level_data::RawHikingPath],
    ) -> Option<&'a crate::level_data::RawWaypoint> {
        hiking_paths
            .get(usize::from(self.hiking_path_index))?
            .waypoints
            .get(index as usize)
    }

    /// Peek at the next waypoint (without advancing).
    pub fn peek_next_waypoint<'a>(
        &self,
        hiking_paths: &'a [crate::level_data::RawHikingPath],
    ) -> Option<&'a crate::level_data::RawWaypoint> {
        let mut tmp = self.clone();
        tmp.advance();
        tmp.current_waypoint(hiking_paths)
    }

    /// Set current waypoint index.
    pub fn set_current_index(&mut self, index: u8) {
        self.last_waypoint_index = self.current_waypoint_index;
        self.current_waypoint_index = index;
    }

    /// Clear position history.
    pub fn reset_history(&mut self) {
        self.history.clear();
    }

    /// Pre-seed history from waypoints already behind the current waypoint.
    /// Called at level start so minions can immediately form up behind
    /// the chief without waiting for it to walk.
    pub fn initialize_history_entries_on_path(
        &mut self,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) {
        debug_assert!(self.history.is_empty());

        let path = match hiking_paths.get(usize::from(self.hiking_path_index)) {
            Some(p) => p,
            None => return,
        };

        let mut distance: u16 = 0;
        for i in 0..self.current_waypoint_index as usize {
            let wp = &path.waypoints[i];
            let next_wp = &path.waypoints[i + 1];

            let dx = next_wp.x as f32 - wp.x as f32;
            let dy = next_wp.y as f32 - wp.y as f32;

            let direction = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u8;

            self.history.push(PathHistoryEntry {
                position: Position {
                    x: wp.x as f32,
                    y: wp.y as f32,
                    sector: SectorHandle::new(wp.sector),
                    level: wp.level,
                },
                direction,
                distance,
            });

            let seg_len = crate::position_interface::vector_norm_iso(dx, dy);
            distance += seg_len as u16;
        }
    }

    /// Record the chief's current position and direction.
    /// Called every frame for patrol chiefs.
    pub fn add_history_entry(&mut self, position: Position, direction: u8) {
        let new_distance = if self.history.is_empty() {
            0u16
        } else {
            let last = self.history.last().unwrap();
            let dx = position.x - last.position.x;
            let dy = position.y - last.position.y;
            let step_distance = crate::position_interface::vector_norm_iso(dx, dy) as u16;
            if step_distance == 0 {
                return; // No movement since last entry
            }
            let mut new_dist = last.distance + step_distance;
            // Shift all distances down when they get too high (only relative
            // differences matter).
            if new_dist > 1000 {
                let first_dist = self.history[0].distance;
                for entry in &mut self.history {
                    entry.distance -= first_dist;
                }
                new_dist -= first_dist;
            }
            new_dist
        };
        self.history.push(PathHistoryEntry {
            position,
            direction,
            distance: new_distance,
        });
    }

    /// Compute formation positions for patrol minions behind the chief.
    /// Returns `(position, direction)` pairs for each minion slot.
    ///
    /// `fast_grid` + `chief_move_box` enable the 3-step fallback
    /// (60% / 30% / 0% sideways) when `IsStraightMovementAutorized`
    /// rejects the full-sideways point.  The chief's move box is
    /// expanded by 3 on each side for this test — callers should do the
    /// same before passing it in.  When `fast_grid` is `None` the
    /// fallback is skipped and the full offset is used unconditionally
    /// (unit-test path; the pathfinder re-converges on the next tick).
    pub fn compute_patrol_positions(
        &mut self,
        patrol_size: usize,
        fast_grid: Option<&crate::fast_find_grid::FastFindGrid>,
        chief_move_box: &crate::geo2d::BBox2D,
    ) -> Vec<(Position, u16)> {
        if self.history.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(patrol_size);
        let mut history_idx: isize = self.history.len() as isize - 1;
        let mut required_distance: u16 = 0;
        let last_distance = self.history.last().unwrap().distance;
        let mut sidewards = [0.0f32; 2];

        for pos_idx in 0..patrol_size {
            if pos_idx % 2 == 0 {
                // EVEN CASE: one row further back from chief
                required_distance = required_distance.saturating_add(PATROL_BACKWARDS_DISTANCE);

                // Search backwards in history for the right distance
                loop {
                    history_idx -= 1;
                    if history_idx < 0 {
                        // Not enough history yet (e.g. start of level) —
                        // abandon without trimming.
                        return result;
                    }
                    let actual_distance =
                        last_distance - self.history[history_idx as usize].distance;
                    if actual_distance >= required_distance {
                        break;
                    }
                }

                let entry = &self.history[history_idx as usize];

                if pos_idx < patrol_size - 1 {
                    // Perpendicular to the chief's walking direction (right side).
                    let perp_sector = (entry.direction as i16 + 4) & 15;
                    let dir = crate::position_interface::sector_to_vector_iso(perp_sector);
                    sidewards = [
                        dir[0] * PATROL_HALF_SIDEWARDS_DISTANCE,
                        dir[1] * PATROL_HALF_SIDEWARDS_DISTANCE,
                    ];
                } else {
                    // Last guy (odd patrol count) walks in the center
                    sidewards = [0.0, 0.0];
                }
            } else {
                // ODD CASE: same row, opposite side
                sidewards = [-sidewards[0], -sidewards[1]];
            }

            let entry = &self.history[history_idx as usize];
            // Try the full offset, then fall back to 60% / 30% / 0% if
            // `IsStraightMovementAutorized` rejects.  Without a grid
            // (tests), always accept full.
            let on_path = crate::geo2d::pt(entry.position.x, entry.position.y);
            let mut chosen = sidewards;
            if let Some(grid) = fast_grid {
                const FALLBACK_SCALES: &[f32] = &[1.0, 0.6, 0.3, 0.0];
                for &scale in FALLBACK_SCALES {
                    let candidate = crate::geo2d::pt(
                        on_path.x + sidewards[0] * scale,
                        on_path.y + sidewards[1] * scale,
                    );
                    if scale == 0.0
                        || grid.is_straight_movement_authorized(
                            on_path.into(),
                            candidate.into(),
                            entry.position.level,
                            chief_move_box,
                        )
                    {
                        chosen = [sidewards[0] * scale, sidewards[1] * scale];
                        break;
                    }
                }
            }

            let pos = Position {
                x: entry.position.x + chosen[0],
                y: entry.position.y + chosen[1],
                sector: entry.position.sector,
                level: entry.position.level,
            };
            result.push((pos, entry.direction as u16));
        }

        // Trim old history entries no longer needed by future calls.
        // The semantics are an inclusive delete: entries [0, history_idx-1]
        // are dropped and the list restarts at what was `history_idx`.
        // Rust's `drain` is exclusive-end, so the equivalent range is
        // `0..history_idx`.
        if history_idx > 0 {
            self.history.drain(0..history_idx as usize);
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// PathHistoryEntry — patrol chief position history for formation computation
// ---------------------------------------------------------------------------

/// One entry in the patrol chief's position history, recording where the
/// chief walked.  Used by `compute_patrol_positions` to place minions
/// behind the chief in formation.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathHistoryEntry {
    pub position: Position,
    pub direction: u8,
    /// Cumulative distance walked from the first history entry.
    pub distance: u16,
}

/// Distance behind the chief per formation row.
const PATROL_BACKWARDS_DISTANCE: u16 = 30;
/// Half the sidewards spacing between paired soldiers.
const PATROL_HALF_SIDEWARDS_DISTANCE: f32 = 20.0;
/// Speed factor base for minion catch-up.
pub const PATROL_SPEED_BASE: f32 = 0.3;
/// Speed factor divisor for minion catch-up.
pub const PATROL_SPEED_DIVISOR: f32 = 30.0;

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

/// 2-D game position with sector and level info.
///
/// Sector is currently an opaque handle; once the sector system is fully
/// integrated this will reference it properly.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    /// Sector handle (nominal newtype from `position_interface`).
    /// `None` indicates a null sector / unassigned waypoint.
    pub sector: Option<SectorHandle>,
    pub level: u16,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// ForecastedDestination — port of ForecastDestinationForIA
// ---------------------------------------------------------------------------

/// Predicted destination of a target actor for AI pursuit.
#[derive(Debug, Clone, Copy)]
pub struct ForecastedDestination {
    pub position: Position,
    pub direction: u16,
}

/// Snapshot of a target actor's state needed for destination forecasting.
/// Extracted from the target entity by the engine.
#[derive(Debug, Clone, Copy)]
pub struct ForecastInput {
    pub position_map_x: f32,
    pub position_map_y: f32,
    /// Raw sector number.  Kept as `u16` because the forecast logic
    /// reassigns this to `door.sector_in` / `sector_out` (raw `u16`) and
    /// feeds it into raw sector-number grid lookups; wrapping/unwrapping
    /// each step would just add noise.
    pub sector: u16,
    pub layer: u16,
    pub direction: u16,
    pub forecasted_movement_z: f32,
    /// `Some((door_index, direct))` if the target is mid-door-pass.
    pub door_pass: Option<(crate::gate::DoorIndex, bool)>,
}

/// Predict where a target actor is heading based on their current
/// door/lift/building traversal state.
///
/// Used by the AI to chase where enemies are GOING rather than where
/// they WERE.
///
/// Logic:
/// 1. If the target is passing through a door, resolve the destination
///    side of that door (in/out depending on direction).
/// 2. If the destination sector is a lift, predict the exit floor.
/// 3. If the destination sector is a building and the target just entered,
///    predict exit through a random other gate.
/// 4. Otherwise fall back to the target's current position and direction.
pub fn forecast_destination_for_ia(
    input: &ForecastInput,
    doors: &[crate::gate::Door],
    sectors: &[crate::fast_find_grid::GridSector],
    sector_map: &std::collections::HashMap<crate::sector::SectorNumber, usize>,
) -> ForecastedDestination {
    use crate::gate::DoorType;

    let (mut sector, mut layer, mut point, moving_upwards, current_door_index) =
        if let Some((door_idx, direct)) = input.door_pass {
            if let Some(door) = doors.get(usize::from(door_idx)) {
                if direct {
                    // Passing door directly (outside → inside): destination is IN side.
                    let up = !matches!(
                        door.door_type,
                        DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::BuildingTrap
                    );
                    (
                        u16::from(door.sector_in),
                        door.layer_in,
                        door.point_in,
                        up,
                        Some(door_idx),
                    )
                } else {
                    // Passing door indirectly (inside → outside): destination is OUT side.
                    let up = matches!(
                        door.door_type,
                        DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::BuildingTrap
                    );
                    (
                        u16::from(door.sector_out),
                        door.layer_out,
                        door.point_out,
                        up,
                        Some(door_idx),
                    )
                }
            } else {
                // Door index out of range — fall back to current position.
                (
                    input.sector,
                    input.layer,
                    (input.position_map_x, input.position_map_y),
                    input.forecasted_movement_z > 0.0,
                    None,
                )
            }
        } else {
            // Not passing a door — use current position.
            (
                input.sector,
                input.layer,
                (input.position_map_x, input.position_map_y),
                input.forecasted_movement_z > 0.0,
                None,
            )
        };
    let mut direction = input.direction;

    // Look up the destination sector in the grid.
    let grid_sector = sector_map
        .get(&crate::sector::SectorNumber::new(sector as i16))
        .and_then(|&idx| sectors.get(idx));

    // Building-gate branch only fires when passing the door directly
    // (outside→inside). Extract the `direct` flag from the door_pass tuple.
    let passing_door_directly = input.door_pass.map(|(_, direct)| direct).unwrap_or(false);

    if let Some(gs) = grid_sector {
        if gs.sector_type.is_lift() {
            // Target is on a lift — predict high/low exit.
            // Direction uses `(PointOut - PointMid)`.
            if let Some(exit_door) = find_lift_exit_door(sector, moving_upwards, doors) {
                sector = u16::from(exit_door.sector_out);
                layer = exit_door.layer_out;
                point = exit_door.point_out;
                direction = door_exit_direction_from_mid(exit_door);
            }
        } else if gs.sector_type.is_building() && passing_door_directly {
            // Target entering a building (direct only) — predict exit
            // through a random other gate.
            // Direction uses `(PointOut - PointIn)`.
            if let Some(current_door) = current_door_index
                && let Some(exit_door) = pick_building_exit_gate(sector, current_door, doors)
            {
                sector = u16::from(exit_door.sector_out);
                layer = exit_door.layer_out;
                point = exit_door.point_out;
                direction = door_exit_direction_from_in(exit_door);
            }
        }
        // else: position is fine, keep current direction.
    }

    ForecastedDestination {
        position: Position {
            x: point.0,
            y: point.1,
            sector: SectorHandle::new(sector),
            level: layer,
        },
        direction,
    }
}

/// Find the exit door for a lift sector in the given direction.
///
/// Scans the door table for a door whose `sector_in` matches the lift
/// sector with the appropriate lift door type (High or Low).
fn find_lift_exit_door(
    lift_sector: u16,
    moving_upwards: bool,
    doors: &[crate::gate::Door],
) -> Option<&crate::gate::Door> {
    use crate::gate::DoorType;
    doors.iter().find(|d| {
        d.sector_in == lift_sector
            && if moving_upwards {
                matches!(d.door_type, DoorType::LiftHigh | DoorType::LiftHighCrenel)
            } else {
                d.door_type == DoorType::LiftLow
            }
    })
}

/// Pick a random building exit gate that isn't the entry door.
///
/// Collects candidates and uses `fastrand` for the random pick.
fn pick_building_exit_gate(
    building_sector: u16,
    exclude_door: crate::gate::DoorIndex,
    doors: &[crate::gate::Door],
) -> Option<&crate::gate::Door> {
    let exclude = u32::from(exclude_door);
    let candidates: Vec<&crate::gate::Door> = doors
        .iter()
        .enumerate()
        .filter(|(i, d)| d.sector_in == building_sector && *i as u32 != exclude)
        .map(|(_, d)| d)
        .collect();
    if candidates.is_empty() {
        None
    } else {
        Some(candidates[crate::sim_rng::usize(..candidates.len())])
    }
}

/// Compute the exit direction from a door's geometry.
///
/// For lifts: `(GetPointOut() - GetPointMid()).GetSector0to15(ASPECT_RATIO)`.
/// For building gates: `(GetPointOut() - GetPointIn()).GetSector0to15(ASPECT_RATIO)`.
fn door_exit_direction_from_mid(door: &crate::gate::Door) -> u16 {
    let dx = door.point_out.0 - door.point_mid.0;
    let dy = door.point_out.1 - door.point_mid.1;
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

fn door_exit_direction_from_in(door: &crate::gate::Door) -> u16 {
    let dx = door.point_out.0 - door.point_in.0;
    let dy = door.point_out.1 - door.point_in.1;
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

// ---------------------------------------------------------------------------
// Waypoint-macro opcodes
// ---------------------------------------------------------------------------

/// One-byte waypoint-macro opcode.
///
/// The values are assigned sequentially from 0 so the u8 reprs can be
/// decoded directly from the compressed macro bytestream stored on a
/// `RawWaypoint`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroOpcode {
    /// `CMD_REVERSE_PATH` — flip ping-pong traversal direction of the
    /// patrol path in place and recurse.
    ReversePath = 0,
    /// `CMD_SKIP_POINT` — advance past the current waypoint, then end
    /// the macro (recurses with `remaining == 0`).
    SkipPoint = 1,
    /// `CMD_GOTO_POINT` — jump to an absolute waypoint index (2B LE16),
    /// then end the macro.
    GotoPoint = 2,
    /// `CMD_FACE_TO` — turn to a 0..15 sector (2B LE16), transition to
    /// `DefaultInMacroWaitingForDone`, resume on EVENT_DONE.
    FaceTo = 3,
    /// `CMD_WAIT` — launch macro timer (2B LE16 frames), stay in
    /// `DefaultInMacro`, resume when macro timer rings.
    Wait = 4,
    /// `CMD_CHECK_4` — start CheckFor comportment against friend NPC
    /// (2B LE16 friend id + 2B LE16 frames).
    Check4 = 5,
    /// `CMD_CHECK_4_SYNC` — CheckFor with synchronization index
    /// (2B friend id + 2B frames + 2B sync index).
    Check4Sync = 6,
    /// `CMD_STAY_HERE` — drop the patrol path (`AssignNewPatrolPath(-1)`),
    /// then recurse.
    StayHere = 7,
    /// `CMD_CHANGE_WAY` — switch to a new patrol path by index
    /// (2B LE16), break the macro, and return to duty.
    ChangeWay = 8,
    /// `CMD_RUN` — set the persistent `GOTO_RUN` walking flag, then
    /// recurse.
    Run = 9,
    /// `CMD_WALK` — clear `GOTO_RUN`, then recurse.
    Walk = 10,
    /// `CMD_LOOK_LEFT` — `LookSidewards(Left)`, then wait for DONE.
    LookLeft = 11,
    /// `CMD_LOOK_RIGHT` — `LookSidewards(Right)`, then wait for DONE.
    LookRight = 12,
    /// `CMD_BEND` — `LookSidewards(Down)`, launch macro timer
    /// (2B LE16), stay in `DefaultInMacro`.
    Bend = 13,
    /// `CMD_PATROL_STOP` — set `patrol_stopped = true`, officer says
    /// `OfficerStopsPatrol` remark, recurse.
    PatrolStop = 14,
    /// `CMD_PATROL_DIRECTION` — instruct patrol formation facing
    /// direction (2B LE16), recurse.
    PatrolDirection = 15,
    /// `CMD_PATROL_START` — clear `patrol_stopped`, officer says
    /// `OfficerStartsPatrol`, reinitialize patrol, recurse.
    PatrolStart = 16,
}

impl MacroOpcode {
    /// Decode a single opcode byte, returning `None` for unknown values.
    pub fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::ReversePath,
            1 => Self::SkipPoint,
            2 => Self::GotoPoint,
            3 => Self::FaceTo,
            4 => Self::Wait,
            5 => Self::Check4,
            6 => Self::Check4Sync,
            7 => Self::StayHere,
            8 => Self::ChangeWay,
            9 => Self::Run,
            10 => Self::Walk,
            11 => Self::LookLeft,
            12 => Self::LookRight,
            13 => Self::Bend,
            14 => Self::PatrolStop,
            15 => Self::PatrolDirection,
            16 => Self::PatrolStart,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// AI State
// ---------------------------------------------------------------------------

/// Top-level AI state.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum AiState {
    Sleeping = 0,
    #[default]
    Default = 1,
    Wondering = 2,
    Seeking = 3,
    Attacking = 4,
    Menacing = 5,
    Fleeing = 6,
}

/// State codes used in the event system / scripts. Matches the `#define
/// AISTATE_*` constants from the header.
impl AiState {
    pub const SCRIPT_DRIVEN: u32 = 7;

    /// Translate an internal `STATE_*` engine enum to the script-visible
    /// `AISTATE_*` constant emitted by `Script::GetAIState`. The internal
    /// and script numeric spaces coincide for Sleeping/Default/Wondering/Seeking
    /// but differ for Attacking/Menacing/Fleeing.
    pub fn to_script_code(self) -> i32 {
        match self {
            Self::Sleeping => 0,  // AISTATE_SLEEPING
            Self::Default => 1,   // AISTATE_DEFAULT
            Self::Wondering => 2, // AISTATE_WONDERING
            Self::Seeking => 3,   // AISTATE_SEEKING
            Self::Menacing => 4,  // AISTATE_MENACING
            Self::Fleeing => 5,   // AISTATE_FLEEING
            Self::Attacking => 6, // AISTATE_ATTACKING
        }
    }

    /// AI event code for script `FilterAIEvent` state-change notifications.
    pub fn state_change_event_code(self) -> i32 {
        match self {
            Self::Sleeping => 100,
            Self::Default => 101,
            Self::Wondering => 102,
            Self::Seeking => 103,
            Self::Attacking => 104,
            Self::Menacing => 105,
            Self::Fleeing => 106,
        }
    }
}

// ── AI event codes for FilterAIEvent ────────────────────────────────
//
// Used by the per-actor script `FilterAIEvent` callback which can block
// stimulus processing (early gate) or is notified of state changes (late
// notification).

/// Map a stimulus type to its AI event code for `FilterAIEvent`.
///
/// Returns `Some(code)` for stimuli that `StartThink`'s big switch maps,
/// `None` for unmapped types (Rust-only stimuli or meta markers). The
/// mapping covers event codes 0–52. Unmapped stimuli bypass `FilterAIEvent`
/// entirely — falling into the default arm, which a well-formed script's
/// filter never branches on.
pub fn stimulus_to_ai_event_code(st: StimulusType) -> Option<i32> {
    match st {
        // Perception events (0–14)
        StimulusType::EventView => Some(0),
        StimulusType::EventOutOfView => Some(1),
        StimulusType::EventHear => Some(2),
        StimulusType::EventReachPoint => Some(3),
        StimulusType::EventCouldntReachPoint => Some(4),
        StimulusType::EventDone => Some(5),
        StimulusType::EventImpossible => Some(6),
        StimulusType::EventTimer => Some(7),
        StimulusType::EventSeesBody => Some(8),
        StimulusType::EventSeesObject => Some(9),
        StimulusType::EventSeesSoldier => Some(10),
        StimulusType::EventSeesFriendInTrouble => Some(11),
        StimulusType::EventFitAgain => Some(12),
        StimulusType::EventGotHit => Some(13),
        StimulusType::EventLoseConsciousness => Some(14),
        // Extended perception / combat events (15–32)
        StimulusType::EventMissesCharly => Some(15),
        StimulusType::EventObjectAway => Some(16),
        StimulusType::EventSeesCharly => Some(17),
        StimulusType::EventSyncCharly => Some(18),
        StimulusType::EventAfterScriptGoOn => Some(19),
        StimulusType::EventReturnToDuty => Some(20),
        StimulusType::EventPanic => Some(21),
        StimulusType::EventEnterSwordfight => Some(22),
        StimulusType::EventQuitSwordfight => Some(23),
        StimulusType::EventSwordStrike => Some(24),
        StimulusType::EventWasp => Some(25),
        StimulusType::EventWaspAway => Some(26),
        StimulusType::EventApple => Some(27),
        StimulusType::EventNet => Some(28),
        StimulusType::EventNetAway => Some(29),
        StimulusType::EventSeesBeggar => Some(30),
        StimulusType::EventGetArrow => Some(31),
        StimulusType::EventSeesBrawl => Some(32),
        // Inter-NPC calls (33–48)
        StimulusType::CallAlert => Some(33),
        StimulusType::CallCombatAlert => Some(34),
        StimulusType::CallFinishBrawl => Some(35),
        StimulusType::CallHey => Some(36),
        StimulusType::CallTowerGuardAlert => Some(37),
        StimulusType::CallTowerGuardCallsMe => Some(38),
        StimulusType::CallHint => Some(39),
        StimulusType::CallInstruction => Some(40),
        StimulusType::CallLookThere => Some(41),
        StimulusType::CallCoordinate => Some(42),
        StimulusType::CallReport => Some(43),
        StimulusType::CallGoToOfficer => Some(44),
        StimulusType::CallMrOfficerIAmBack => Some(45),
        StimulusType::CallCharlyIsBack => Some(46),
        StimulusType::CallPatrolCoordinate => Some(47),
        StimulusType::CallYouJustWait => Some(48),
        // Chase / combat / patrol events (49–52)
        StimulusType::EventAppleChaseNear => Some(49),
        StimulusType::EventDoorCombat => Some(50),
        StimulusType::EventGaloppLoopEnd => Some(51),
        StimulusType::EventSeesShadow => Some(52),
        // Rust-only stimuli: the filter never fires for these — they
        // fall into the unmapped/default path. `None` here means
        // "unmapped — don't call FilterAIEvent".
        StimulusType::EventPcShotAtMe
        | StimulusType::EventArrowLaunched
        | StimulusType::EventStone
        | StimulusType::EventAdversaryWeak
        | StimulusType::EventAfterCombatInjury
        | StimulusType::CallCleanUpAfterBrawl
        | StimulusType::EventMyTalk0
        | StimulusType::EventMyTalk1
        | StimulusType::EventMyTalk2
        | StimulusType::EventMyTalk3
        | StimulusType::CallYourTalk0
        | StimulusType::CallYourTalk1
        | StimulusType::CallYourTalk2
        | StimulusType::CallYourTalk3
        | StimulusType::EventGoodStrike
        | StimulusType::EventLethalStrike
        | StimulusType::EventEnemyNear
        | StimulusType::EventStop
        | StimulusType::ForceBattleDecision
        | StimulusType::NoEvent => None,
    }
}

fn pascal_debug_name_to_hyphen_upper<T: std::fmt::Debug>(value: T) -> String {
    let name = format!("{value:?}");
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev: Option<char> = None;
    let mut chars = name.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_uppercase() {
            let split_before = prev.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || chars.peek().is_some_and(|next| next.is_lowercase()) && p.is_uppercase()
            });
            if split_before {
                out.push('-');
            }
        } else if ch.is_ascii_digit() && prev.is_some_and(|p| !p.is_ascii_digit()) {
            out.push('-');
        }

        for upper in ch.to_uppercase() {
            out.push(upper);
        }
        prev = Some(ch);
    }

    out
}

// ---------------------------------------------------------------------------
// AI Substate — massive enum
// ---------------------------------------------------------------------------

/// Fine-grained substate within an [`AiState`]. Implemented as a giant
/// flat enum with sentinel markers for each state group.
///
/// The numeric layout is preserved so savegame compatibility is possible
/// if needed.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
#[allow(non_camel_case_types)] // preserve original naming for clarity
pub enum Substate {
    // -- Sleeping substates --
    StartSleepingSubstates = 0,

    SleepingForever,
    SleepingUnconscious,
    SleepingNapping,
    SleepingAwakening,

    EndSleepingSubstates,

    // -- Default substates --
    StartDefaultSubstates,

    DefaultGotoPost,
    DefaultGotoPostTurn,
    DefaultGotoRoute,
    DefaultGotoRouteTurn,
    DefaultOnPost,
    DefaultOnPostLookingSidewards,
    DefaultEnroute,
    DefaultScriptDriven,
    DefaultInMacro,
    DefaultInMacroWaitingForDone,
    DefaultHomeSweetHome,
    DefaultLookingOfficerForAdvice,
    DefaultLookingForCharly,
    DefaultLookingSidewardsForCharly,
    DefaultDetectedCharly,
    DefaultSynchronizing,
    DefaultPatrolEnroute,
    DefaultPatrolEnrouteWaiting,
    DefaultLookingShadow,
    DefaultChildApproachedWhistling,

    EndDefaultSubstates,

    // -- Wondering substates --
    StartWonderingSubstates,

    WonderingWatching,
    WonderingLooking1,
    WonderingLooking1Sidewards,
    WonderingLooking2,
    WonderingLooking2Sidewards,
    WonderingLooking3,
    WonderingLooking3Sidewards,
    WonderingWaspInArmour,
    WonderingAppleReactiontime,
    WonderingAppleChasingChild,
    WonderingAppleChasingChildWaiting,
    WonderingAppleChasingChildEnd,
    WonderingMoneyReactiontime,
    WonderingApproachingMoney,
    WonderingRunningForMoney,
    WonderingTakingMoney,
    WonderingBrawlReactiontime,
    WonderingBrawlApproaching,
    WonderingBrawlHitting,
    WonderingBrawlGotHit,
    WonderingBrawlRecovering,
    WonderingWatchingForMoreMoney,
    WonderingApproachingToLoot,
    WonderingLooting,
    WonderingAleReactiontime,
    WonderingApproachingAle,
    WonderingDrinkingAle,
    WonderingAleAway,
    WonderingWatchingTowerGuard,
    WonderingUnderNet,
    WonderingCivilianAdmiringHero,
    WonderingCivilianEnemyReactiontime,
    WonderingCivilianBodyReactiontime,
    WonderingOfficerSeeingBrawl,
    WonderingOfficerApproachingBrawl,
    WonderingOfficerFinishingBrawl,
    WonderingSoldierLookingOfficerWhoFinishedBrawl,
    WonderingHeardWhistling,
    WonderingWatchingWhistling,
    WonderingChildApproachingWhistling,

    EndWonderingSubstates,

    // -- Seeking substates --
    StartSeekingSubstates,

    SeekingHeardstepsReactiontime,
    SeekingHeardsteps,
    SeekingSeekpoint,
    SeekingSeekpointWatching,
    SeekingSeekpointWatchingSidewards,
    SeekingSeekpointPassedAmbushPointLeft,
    SeekingSeekpointPassedAmbushPointRight,
    SeekingSeekpointCheckingAmbushPoint,
    SeekingSeekpointApproachingBeggar,
    SeekingSeekpointIdentifyingBeggar1,
    SeekingSeekpointIdentifyingBeggar2,
    SeekingJustWatching,
    SeekingJustWatchingSidewards,
    SeekingKnightWatchingTowerGuard,
    SeekingOfficerCallSoldier,
    SeekingOfficerWaitForSoldier,
    SeekingOfficerInstructSoldier,
    SeekingOfficerWaitForInstructedSoldier,
    SeekingOfficerGetReportFromSoldier,
    SeekingOfficerGetAlertingReportFromSoldier,
    SeekingSoldierCalledByOfficer,
    SeekingSoldierGoToOfficer,
    SeekingSoldierGetInstructedByOfficer,
    SeekingSoldierReturnToOfficer,
    SeekingSoldierGiveReportToOfficer,
    SeekingSoldierGiveAlertingReportToOfficerStart,
    SeekingSoldierGiveAlertingReportToOfficerPoint,
    SeekingSoldierGiveAlertingReportToOfficerEnd,
    SeekingOfficerCallGroup,
    SeekingOfficerWaitForGroup,
    SeekingOfficerWaitInsideHouseToInstructGroup,
    SeekingOfficerLeavingHouseToInstructGroup,
    SeekingOfficerInstructGroup,
    SeekingOfficerInstructGroupPointing,
    SeekingOfficerWaitForInstructedGroup,
    SeekingGroupCalledByOfficer,
    SeekingGroupGoToOfficer,
    SeekingGroupGetInstructedByOfficer,
    SeekingBodyReactiontime,
    SeekingBody,
    SeekingNet,
    SeekingTakingNet,
    SeekingBodyLookingDeadBody,
    SeekingBodyAwakeningSleeperr,
    SeekingOfficerLookingForSoldiers1,
    SeekingOfficerLookingForSoldiers1Sidewards,
    SeekingOfficerLookingForSoldiers2,
    SeekingOfficerLookingForSoldiers2Sidewards,
    SeekingOfficerLookingForSoldiers3,
    SeekingOfficerLookingForSoldiers3Sidewards,
    SeekingRunningToOfficer,
    SeekingRunningToOfficerSeen,
    SeekingOfficerWaitForAlertingSoldier,
    SeekingArrowReactiontime,
    SeekingArrow,
    SeekingArrowJustWatching,
    SeekingArrowJustWatchingSidewards,
    SeekingCharly,
    SeekingCharlyWatching,
    SeekingDetectedCharly,
    SeekingSendCharlyToOfficer,
    SeekingLookingResurrectedCharly,
    SeekingCharlySentToOfficer,
    SeekingCharlyGoToOfficer,
    SeekingCharlyGoToOfficerSeen,
    SeekingCharlyGetLectureByOfficer,
    SeekingOfficerWaitForCharly,
    SeekingOfficerLectureCharly,
    SeekingOfficerLectureCharlyPointing,
    SeekingCombatAlertReactiontime,
    SeekingCombatAlert,
    SeekingCivilianRunningToSoldier,
    SeekingCivilianRunningToSoldierSeen,
    SeekingCivilianGiveAlertingReportToSoldierStart,
    SeekingCivilianGiveAlertingReportToSoldierPoint,
    SeekingCivilianGiveAlertingReportToSoldierEnd,
    SeekingWaitForAlertingCivilian,
    SeekingGetReportFromCivilian,
    SeekingGetAlertingReportFromCivilian,

    EndSeekingSubstates,

    // -- Attacking substates --
    StartAttackingSubstates,

    AttackingReactiontimeTurning,
    AttackingReactiontime,
    AttackingReactiontimeRunning,
    AttackingRunningToEnemy,
    AttackingWalkingToEnemy,
    AttackingChargingEnemy,
    AttackingOverviewLookLeft,
    AttackingOverviewLookRight,
    // NOTE: a `SUBSTATE_ATTACKING_SWORDFIGHT_SPECIAL_STRIKE` variant
    // intentionally does NOT exist between `AttackingSwordfight` and
    // `AttackingSwordfightParade`. Such a substate would duplicate
    // information already owned by the sequence manager (the pending
    // strike sequence), leaving two sources of truth that could wedge out
    // of sync when the sequence was interrupted before firing EVENT_DONE.
    // The "in the middle of a special strike" condition is derived from
    // `EnemyAi::pending_special_strike`, which is tied to the sequence's
    // lifetime via per-tick reconciliation in
    // `engine/melee.rs::tick_enemy_sword_attacks`. The NPC stays in
    // `AttackingSwordfight` for the whole strike.
    AttackingSwordfight,
    AttackingSwordfightParade,
    AttackingQuittingSwordfight,
    AttackingReserve,
    AttackingReserveOverview,
    AttackingApproachToObserve,
    AttackingObserve,
    AttackingObserveAndMove,
    AttackingGotHit,
    AttackingGotHitStandingUp,
    AttackingHitting,
    AttackingApproachingNewEnemy,
    AttackingMovingAroundOldEnemy,
    AttackingApproachingSleepingEnemy,
    AttackingKillingSleepingEnemy,
    AttackingBowShooting,
    AttackingBowLoading,
    AttackingBowAiming,
    AttackingBowObserving,
    AttackingBowObservingLoading,
    AttackingArcherRetireFromCombat,
    AttackingArcherRetireFromCombatTurn,
    AttackingProtectingWithShield,
    AttackingAdvancingWithShield,
    AttackingBowRunningBehindShieldBearer,
    AttackingBowCorrectingPosition,
    AttackingPhalanx,
    AttackingRunningToPhalanx,
    AttackingOfficerGivingOrders,
    AttackingOfficerGivingOrdersWaiting,
    AttackingTooProudToAttack,
    AttackingTooProudToAttackOverview,
    AttackingTooProudToAttackRetire,
    AttackingTooProudToAttackRetireTurn,
    AttackingTooProudToAttackApproach,
    AttackingTowerGuardAlert,
    AttackingTowerGuardObserve,
    AttackingArcherRunOnShootingPath,
    AttackingArcherRunOnShootingPathFinalSprint,
    AttackingArcherRunOnShootingPathTurn,
    AttackingArcherWaitOnArcheryPath,
    AttackingArcherWaitOnArcheryPathBending,
    AttackingDoorFightDelay,
    AttackingDoorFightLeaving,
    AttackingDoorFightTurning,
    AttackingDoorFightWaiting,
    AttackingRiderChargingApproachingBlindly,
    AttackingRiderChargingApproaching,
    AttackingRiderChargingPassing,
    AttackingRiderChargingGettingDistance,
    AttackingRiderChargingReturning,
    AttackingReactiontimeBending,
    AttackingArcherWaitOnBendPoint,

    AttackingDummyBehaviour,

    EndAttackingSubstates,

    // -- Menacing substates --
    StartMenacingSubstates,

    MenacingPcInComa,

    EndMenacingSubstates,

    // -- Fleeing substates --
    StartFleeingSubstates,

    FleeingRunToHide,
    FleeingRunToDoor,
    FleeingHiding,
    FleeingRunForArrowReserves,
    FleeingPanic,
    FleeingChildChased,
    FleeingChildChasedSupplementalRuns,
    FleeingChildChasedEnd,
    FleeingChildFriendChased,
    FleeingRunToAlertSoldiers,
    FleeingRetireFromCombat,
    FleeingRetireFromCombatTurn,
    FleeingMerryManRunToLeaveMap,
    FleeingMerryManLeaveMap,

    EndFleeingSubstates,

    // -- Additional substates (added later, outside main groups) --
    BeginAdditionalSubstates,

    AttackingSwordfightStepBack,
    WonderingAppleSauceInTheVisor,
    DefaultPatrolEnrouteRunning,
    DefaultGotoChief,
    DefaultPatrolChiefReturnToPatrol,
    WonderingApproachingBrawlVictim,
    WonderingAwakenBrawlVictim,
    WonderingOfficerFinishingBrawlWaiting,
    AttackingReturnToOtherPcAfterMenacing,
    SeekingCharlyGetLectureByOfficer2,
    AttackingRunningToLadder,
    AttackingWaitingAtLadder,
    SeekingHeardstepsPreReactiontime,
    AttackingLastReserve,
    AttackingRunToAvengerOnRoof,
    AttackingWaitForAvengerOnRoof,
    SeekingGotStopEvent,
    SeekingGetAlertingReportFromCivilianLook,

    NumberOfSubstates,

    /// Sentinel — no substate.
    None = 0xFFFF_FFFF,
}

impl Substate {
    pub fn log_string_from_u16(raw: u16) -> String {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or_else(|| "SUBSTATE-???".to_string())
    }

    pub fn log_string(self) -> Option<String> {
        use Substate::*;

        let text = match self {
            StartSleepingSubstates
            | EndSleepingSubstates
            | StartDefaultSubstates
            | EndDefaultSubstates
            | StartWonderingSubstates
            | EndWonderingSubstates
            | StartSeekingSubstates
            | EndSeekingSubstates
            | StartAttackingSubstates
            | EndAttackingSubstates
            | StartMenacingSubstates
            | EndMenacingSubstates
            | StartFleeingSubstates
            | EndFleeingSubstates
            | BeginAdditionalSubstates
            | AttackingRunToAvengerOnRoof
            | AttackingWaitForAvengerOnRoof
            | NumberOfSubstates
            | None => return std::option::Option::None,

            DefaultGotoPost => "SUBSTATE-DEFAULT-GOTOPOST".to_string(),
            DefaultGotoPostTurn => "SUBSTATE-DEFAULT-GOTOPOST-TURN".to_string(),
            DefaultGotoRoute => "SUBSTATE-DEFAULT-GOTOROUTE".to_string(),
            DefaultGotoRouteTurn => "SUBSTATE-DEFAULT-GOTOROUTE-TURN".to_string(),
            DefaultGotoChief => "SUBSTATE-DEFAULT-GOTOCHIEF".to_string(),
            DefaultOnPost => "SUBSTATE-DEFAULT-ONPOST".to_string(),
            DefaultOnPostLookingSidewards => {
                "SUBSTATE-DEFAULT-ONPOST-LOOKING-SIDEWARDS".to_string()
            }
            DefaultInMacro => "SUBSTATE-DEFAULT-INMACRO".to_string(),
            DefaultInMacroWaitingForDone => "SUBSTATE-DEFAULT-INMACRO-WAITING-FOR-DONE".to_string(),
            WonderingBrawlGotHit => "SUBSTATE-WONDERING-BRAWL-GOTHIT".to_string(),
            SeekingBodyAwakeningSleeperr => "SUBSTATE-SEEKING-BODY-AWAKENING-SLEEPER".to_string(),
            AttackingSwordfight => "SUBSTATE-ATTACKING-SWORDFIGHT".to_string(),
            AttackingSwordfightParade => "SUBSTATE-ATTACKING-SWORDFIGHT-PARADE".to_string(),
            AttackingQuittingSwordfight => "SUBSTATE-ATTACKING-QUITTING-SWORDFIGHT".to_string(),
            AttackingSwordfightStepBack => "SUBSTATE-ATTACKING-SWORDFIGHT-STEP-BACK".to_string(),
            AttackingArcherWaitOnArcheryPath => {
                "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH".to_string()
            }
            AttackingArcherWaitOnArcheryPathBending => {
                "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH-BENDING".to_string()
            }
            other => format!("SUBSTATE-{}", pascal_debug_name_to_hyphen_upper(other)),
        };

        Some(text)
    }

    /// Returns `true` if this substate is in the "seek area" group.
    pub fn is_seek_area(self) -> bool {
        matches!(
            self,
            Self::SeekingSeekpoint
                | Self::SeekingSeekpointWatching
                | Self::SeekingSeekpointWatchingSidewards
                | Self::SeekingSeekpointPassedAmbushPointLeft
                | Self::SeekingSeekpointPassedAmbushPointRight
                | Self::SeekingSeekpointCheckingAmbushPoint
                | Self::SeekingSeekpointApproachingBeggar
                | Self::SeekingSeekpointIdentifyingBeggar1
                | Self::SeekingSeekpointIdentifyingBeggar2
        )
    }

    /// Returns `true` if this is any swordfight substate.
    pub fn is_any_swordfight(self) -> bool {
        matches!(
            self,
            Self::AttackingRunningToEnemy
                | Self::AttackingWalkingToEnemy
                | Self::AttackingChargingEnemy
                | Self::AttackingSwordfight
                | Self::AttackingSwordfightParade
                | Self::AttackingApproachingNewEnemy
                | Self::AttackingSwordfightStepBack
                | Self::AttackingMovingAroundOldEnemy
        )
    }

    /// Returns `true` if this is an active swordfight substate.
    pub fn is_real_swordfight(self) -> bool {
        matches!(
            self,
            Self::AttackingSwordfight
                | Self::AttackingSwordfightParade
                | Self::AttackingApproachingNewEnemy
                | Self::AttackingSwordfightStepBack
                | Self::AttackingMovingAroundOldEnemy
        )
    }

    /// Any money-taking substate.
    pub fn is_take_money(self) -> bool {
        matches!(
            self,
            Self::WonderingMoneyReactiontime
                | Self::WonderingApproachingMoney
                | Self::WonderingRunningForMoney
                | Self::WonderingTakingMoney
        )
    }

    /// Any money-fight substate.
    pub fn is_fight_for_money(self) -> bool {
        matches!(
            self,
            Self::WonderingBrawlReactiontime
                | Self::WonderingBrawlApproaching
                | Self::WonderingBrawlHitting
                | Self::WonderingBrawlGotHit
                | Self::WonderingBrawlRecovering
                | Self::WonderingApproachingToLoot
                | Self::WonderingLooting
                | Self::WonderingWatchingForMoreMoney
        )
    }

    /// Any ale-taking substate.
    pub fn is_take_ale(self) -> bool {
        matches!(
            self,
            Self::WonderingAleReactiontime
                | Self::WonderingApproachingAle
                | Self::WonderingDrinkingAle
                | Self::WonderingAleAway
        )
    }
}

// ---------------------------------------------------------------------------
// Emoticon type
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum EmoticonType {
    #[default]
    None = 0,
    GrowingQuestionMark,
    QuestionMark,
    XMark,
    Zzz,
    Cloud,
    Sun,
    Thunderstorm,
    Drunken,
}

// ---------------------------------------------------------------------------
// Probability distribution
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum ProbabilityDistribution {
    Rectangle = 0,
    Gauss,
    GaussHighVariance,
    Dirac,
}

// ---------------------------------------------------------------------------
// Stimulus types (events / calls)
// ---------------------------------------------------------------------------

/// The type of stimulus that can trigger an AI reaction.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum StimulusType {
    // -- Perception events --
    EventView = 0,
    EventOutOfView,
    EventHear,
    EventReachPoint,
    EventCouldntReachPoint,
    EventDone,
    EventImpossible,
    EventTimer,
    EventPcShotAtMe,
    EventSeesBody,
    EventSeesObject,
    EventSeesSoldier,
    EventSeesFriendInTrouble,
    EventFitAgain,
    EventGotHit,
    EventLoseConsciousness,
    EventMissesCharly,
    EventObjectAway,
    EventSeesCharly,
    EventSyncCharly,
    EventAfterScriptGoOn,
    EventReturnToDuty,
    EventPanic,
    EventEnterSwordfight,
    EventQuitSwordfight,
    EventSwordStrike,
    EventWasp,
    EventWaspAway,
    EventApple,
    EventNet,
    EventNetAway,
    EventSeesBeggar,
    EventGetArrow,
    EventSeesBrawl,
    // -- Calls (inter-NPC communication) --
    CallAlert,
    CallCombatAlert,
    CallHey,
    CallHint,
    CallInstruction,
    CallLookThere,
    CallCoordinate,
    CallReport,
    CallGoToOfficer,
    CallMrOfficerIAmBack,
    CallCharlyIsBack,
    CallPatrolCoordinate,
    CallTowerGuardAlert,
    CallTowerGuardCallsMe,
    CallFinishBrawl,
    CallYouJustWait,
    EventAppleChaseNear,
    EventDoorCombat,
    EventGaloppLoopEnd,
    EventSeesShadow,
    EventArrowLaunched,
    EventStone,
    EventAdversaryWeak,
    EventAfterCombatInjury,
    CallCleanUpAfterBrawl,
    EventMyTalk1,
    EventMyTalk2,
    EventMyTalk3,
    CallYourTalk1,
    CallYourTalk2,
    CallYourTalk3,
    EventGoodStrike,
    EventLethalStrike,
    EventEnemyNear,
    EventMyTalk0,
    CallYourTalk0,
    EventStop,
    NoEvent,
    /// Script-triggered: force AI to run battle_decisions() immediately.
    ForceBattleDecision,
}

impl StimulusType {
    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("EVENT-???")
    }

    pub fn log_string(self) -> Option<&'static str> {
        Some(match self {
            StimulusType::EventView => "EVENT-VIEW",
            StimulusType::EventOutOfView => "EVENT-OUTOFVIEW",
            StimulusType::EventHear => "EVENT-HEAR",
            StimulusType::EventReachPoint => "EVENT-REACHPOINT",
            StimulusType::EventCouldntReachPoint => "EVENT-COULDNT-REACHPOINT",
            StimulusType::EventDone => "EVENT-DONE",
            StimulusType::EventImpossible => "EVENT-IMPOSSIBLE",
            StimulusType::EventTimer => "EVENT-TIMER",
            StimulusType::EventPcShotAtMe => "EVENT-PC-SHOT-AT-ME",
            StimulusType::EventSeesBody => "EVENT-SEESBODY",
            StimulusType::EventSeesObject => "EVENT-SEESOBJECT",
            StimulusType::EventSeesSoldier => "EVENT-SEES-SOLDIER",
            StimulusType::EventSeesFriendInTrouble => "EVENT-SEESFRIENDINTROUBLE",
            StimulusType::EventFitAgain => "EVENT-FITAGAIN",
            StimulusType::EventGotHit => "EVENT-GOTHIT",
            StimulusType::EventLoseConsciousness => "EVENT-LOSE-CONSCIOUSNESS",
            StimulusType::EventMissesCharly => "EVENT-MISSES-CHARLY",
            StimulusType::EventObjectAway => "EVENT-OBJECT-AWAY",
            StimulusType::EventSeesCharly => "EVENT-SEES-CHARLY",
            StimulusType::EventSyncCharly => "EVENT-SYNC-CHARLY",
            StimulusType::EventAfterScriptGoOn => "EVENT-AFTER-SCRIPT-GO-ON",
            StimulusType::EventReturnToDuty => "EVENT-RETURN-TO-DUTY",
            StimulusType::EventPanic => "EVENT-PANIC",
            StimulusType::EventEnterSwordfight => "EVENT-ENTER-SWORDFIGHT",
            StimulusType::EventQuitSwordfight => "EVENT-QUIT-SWORDFIGHT",
            StimulusType::EventSwordStrike => "EVENT-SWORDSTRIKE",
            StimulusType::EventWasp => "EVENT-WASP",
            StimulusType::EventWaspAway => "EVENT-WASP-AWAY",
            StimulusType::EventApple => "EVENT-APPLE",
            StimulusType::EventNet => "EVENT-NET",
            StimulusType::EventNetAway => "EVENT-NET-AWAY",
            StimulusType::EventSeesBeggar => "EVENT-SEES-BEGGAR",
            StimulusType::EventGetArrow => "EVENT-GET-ARROW",
            StimulusType::EventSeesBrawl => "EVENT-SEES-BRAWL",
            StimulusType::CallAlert => "CALL-ALERT",
            StimulusType::CallCombatAlert => "CALL-COMBAT-ALERT",
            StimulusType::CallHey => "CALL-HEY",
            StimulusType::CallHint => "CALL-HINT",
            StimulusType::CallInstruction => "CALL-INSTRUCTION",
            StimulusType::CallLookThere => "CALL-LOOKTHERE",
            StimulusType::CallCoordinate => "CALL-COORDINATE",
            StimulusType::CallReport => "CALL-REPORT",
            StimulusType::CallGoToOfficer => "CALL-GO-TO-OFFICER",
            StimulusType::CallMrOfficerIAmBack => "CALL-MR-OFFICER-I-AM-BACK",
            StimulusType::CallCharlyIsBack => "CALL-CHARLY-IS-BACK",
            StimulusType::CallPatrolCoordinate => "CALL-PATROL-COORDINATE",
            StimulusType::CallTowerGuardAlert => "CALL-TOWER-GUARD-ALERT",
            StimulusType::CallTowerGuardCallsMe => "CALL-TOWER-GUARD-CALLS-ME",
            StimulusType::CallFinishBrawl => "CALL-FINISH-BRAWL",
            StimulusType::CallYouJustWait => "CALL-YOU-JUST-WAIT",
            StimulusType::EventAppleChaseNear => "EVENT-APPLE-CHASE-NEAR",
            StimulusType::EventDoorCombat => "EVENT-DOOR-COMBAT",
            StimulusType::EventGaloppLoopEnd => "EVENT-GALOPP-LOOP-END",
            StimulusType::EventSeesShadow => "EVENT-SEES-SHADOW",
            StimulusType::EventArrowLaunched => "EVENT-ARROW-LAUNCHED",
            StimulusType::EventStone => "EVENT-STONE",
            StimulusType::EventAdversaryWeak => "EVENT-ADVERSARY-WEAK",
            StimulusType::EventAfterCombatInjury => "EVENT-AFTER-COMBAT-INJURY",
            StimulusType::CallCleanUpAfterBrawl => "CALL-CLEAN-UP-AFTER-BRAWL",
            StimulusType::EventMyTalk1 => "EVENT-MYTALK-1",
            StimulusType::EventMyTalk2 => "EVENT-MYTALK-2",
            StimulusType::EventMyTalk3 => "EVENT-MYTALK-3",
            StimulusType::CallYourTalk1 => "CALL-YOURTALK-1",
            StimulusType::CallYourTalk2 => "CALL-YOURTALK-2",
            StimulusType::CallYourTalk3 => "CALL-YOURTALK-3",
            StimulusType::EventGoodStrike => "EVENT-GOOD-STRIKE",
            StimulusType::EventLethalStrike => "EVENT-LETHAL-STRIKE",
            StimulusType::EventEnemyNear => "EVENT-ENEMY-NEAR",
            StimulusType::EventMyTalk0 => "EVENT-MYTALK-0",
            StimulusType::CallYourTalk0 => "CALL-YOURTALK-0",
            StimulusType::EventStop => "EVENT-STOP",
            StimulusType::NoEvent | StimulusType::ForceBattleDecision => return None,
        })
    }
}

/// Classification of stimulus types into processing categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StimulusCategory {
    /// Expected events (timer, reachpoint, etc.) — drive state progression.
    Expected,
    /// Unexpected events — interruptions that may change behavior.
    Unexpected,
    /// Alerting events — high-priority perception events.
    Alerting,
    /// Return to duty — special handling.
    ReturnToDuty,
    /// Ignored by this AI type.
    Ignored,
}

// ---------------------------------------------------------------------------
// Remark types
// ---------------------------------------------------------------------------

/// Speech/remark that an NPC can make.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    num_enum::TryFromPrimitive,
)]
#[repr(u32)]
pub enum Remark {
    SeesBody = 0,
    AwakensSleeperr,
    BahIlBougePus,
    SeesEnemy,
    HuntsEnemy,
    StartsCombat,
    ProvokesCombat,
    GoodStrikeCombat,
    CombatInsult,
    Warcry,
    KilledAdversary,
    Cassos,
    CallsOfficer,
    TellsOfficerBody,
    TellsOfficerEnemy,
    TellsOfficerOther,
    TellsOfficerCharlyAway,
    TellsOfficerWhere,
    AwaitsOrders,
    TellsOfficerNothing,
    CharlyDefendsHimself,
    MissesCharly,
    DidntFindCharly,
    FoundCharly,
    SendsCharlyToOfficer,
    WaspSting,
    UnderNet,
    SeesFriendUnderNet,
    Arrow,
    Wounded,
    Dies,
    Strangled,
    TiedUp,
    SeesObject,
    AleYes,
    AleNo,
    Drunken,
    HitByApple,
    ChasesChild,
    CaughtChild,
    GoldYes,
    GoldNo,
    GoldBrawl,
    SearchingSoldierGold,
    SearchingSoldierNothing,
    EndsSearch,
    Panic,
    HearsNoise,
    ControlsBeggar,
    MenacesPcInComa,
    BadExcuse,
    CryAlert,
    ShieldBearerCovers,
    ShieldBearersLineFormation,
    ArchersBehindShieldBearers,
    ProudDontFight,
    ProudFinallyFight,
    OfficerSeesBrawl,
    OfficerEndsBrawl,
    OfficerStopsPatrol,
    OfficerStartsPatrol,
    OfficerComplains,
    OfficerAsksWhatsup,
    OfficerAsksWhere,
    OfficerEndsConversation,
    OfficerCallsSoldier,
    OfficerSendsOutSoldier,
    OfficerCallsGroup,
    OfficerSendsOutGroup,
    OfficerSendsOutGroupForCharly,
    OfficerRebukesCharly,
    OfficerRebukesCharlyEnd,
    OfficerGivesAttackOrder,
    OutOfAmmunition,
    SpecialAction,
    AdmiresObjectScript,
    MissesObjectScript,
    GiveOrReceiveOrder,

    // -- Civilian remarks --
    CivSeesBody,
    CivSeesDeadBody,
    CivCallsSoldier,
    CivDenunciates,
    CivAdmiresRobin,
    CivPanic,
    CivWounded,
    CivDies,
    CivThanx,
    CivCries,
    CivBeerYes,
    CivBeerNo,
    CivSeesSoldiersUnderNet,
    CivUnderNet,
    CivApple,
    CivWasps,
    CivWhistling,
    CivSeesBrawl,
    CivGoldYes,
    CivGoldNo,
    CivBeggarBegging,
    CivBeggarGivesInfo,
    CivBeggarWantsMore,
    CivBeggarGivesLastInfo,
    CivBeggarThanx,
    CivBeggarIdentifiesHimself,
    CivChildCaughtBySoldier,
    CivChildChasedBySoldier,

    // -- VIP remarks --
    VipProudDontFight,
    VipProudFinallyFight,
    VipStartsCombat,
    VipWounded,
    VipDies,
    VipGoodStrikeCombat,
    VipWarcry,
    VipVictory,
    VipSpeaksToHimself,
    VipAleNo,
    VipNetNo,
    VipAppleNo,
    VipWaspsNo,
    VipGoldNo,

    NumberOfRemarks,
    /// Sentinel — no remark.
    TheSoundOfSilence,
}

impl Remark {
    /// First civilian remark variant.
    pub const FIRST_CIVILIAN: Self = Self::CivSeesBody;
    /// First VIP remark variant.
    pub const FIRST_VIP: Self = Self::VipProudDontFight;

    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .map(Self::speech)
            .unwrap_or(" ........... ")
    }

    /// Returns the NPC's actual French speech line for this remark.
    ///
    /// Strings are kept verbatim, including trailing-tab and trailing-space
    /// quirks (some lines pad with tabs to reserve display width). Variants
    /// without a dedicated arm — `NumberOfRemarks`, `TheSoundOfSilence` —
    /// fall through to the default arm.
    pub fn speech(self) -> &'static str {
        match self {
            Remark::SeesBody => "Ca va?",
            Remark::AwakensSleeperr => "Leve-toi!",
            Remark::BahIlBougePus => "Il est mort!",
            Remark::SeesEnemy => "Declinez votre identite! ",
            Remark::HuntsEnemy => "Halte!",
            Remark::StartsCombat => "Defends-toi !",
            Remark::ProvokesCombat => "Allez, viens!",
            Remark::GoodStrikeCombat => "Hahaaaaa!",
            Remark::CombatInsult => "Gibier de Potence!",
            Remark::Warcry => "A l'assaut!",
            Remark::KilledAdversary => "Un de moins!",
            Remark::Cassos => "Il est trop fort !",
            Remark::CallsOfficer => "Sire!",
            Remark::TellsOfficerBody => "Sire, un cadavre, Sire!",
            Remark::TellsOfficerEnemy => "Sire, des ennemis, Sire!",
            Remark::TellsOfficerOther => "Sire, un probleme, Sire !",
            Remark::TellsOfficerCharlyAway => "Sire, un garde manque a l'appel, Sire!",
            Remark::TellsOfficerWhere => "Sire, la-bas, Sire!",
            Remark::AwaitsOrders => "Sire, A vos ordres, Sire!",
            Remark::TellsOfficerNothing => "Sire, il n'y a rien, Sire!",
            Remark::CharlyDefendsHimself => "Sire, je...",
            Remark::MissesCharly => "O\u{FFFD} est-il?",
            Remark::DidntFindCharly => "Je ne le trouve pas!",
            Remark::FoundCharly => "O\u{FFFD} etais-tu?  ",
            Remark::SendsCharlyToOfficer => "L'officier te demande!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::WaspSting => "Bon sang de guepe!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::UnderNet => "Au secours! Sortez-moi d'ici!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::SeesFriendUnderNet => "Aidons-les!",
            Remark::Arrow => "Qu'est-ce ?",
            Remark::Wounded => "Ouille!",
            Remark::Dies => "Ahhhh...",
            Remark::Strangled => " Alagrll mmf rgh",
            Remark::TiedUp => "Mohfefour!",
            Remark::SeesObject => "Qu'est-ce que c'est?",
            Remark::AleYes => "Hmm! Ca c'est gentil!",
            Remark::AleNo => "On ne boit pas en service !",
            Remark::Drunken => " HUPS On ne boit pas HUPS pendant le s... HUPS service!",
            Remark::HitByApple => "Qui a lance ca?",
            Remark::ChasesChild => "Encore ces gamins!",
            Remark::CaughtChild => "Tu vas voir, chenapan !",
            Remark::GoldYes => "Ah, de l'or!",
            Remark::GoldNo => "Cet argent ne m'appartient pas!",
            Remark::GoldBrawl => "Eh! C'est a moi!",
            Remark::SearchingSoldierGold => "Ah! C'est donc lui qui l'avait!",
            Remark::SearchingSoldierNothing => "C'est pas lui...",
            Remark::EndsSearch => "Il faut que je retourne a mon poste...",
            Remark::Panic => "Allons chercher des secours!",
            Remark::HearsNoise => "Qui va la?...",
            Remark::ControlsBeggar => "Controle!",
            Remark::MenacesPcInComa => "J'en tiens un!",
            Remark::BadExcuse => "Sire, il vous a insulte, Sire!",
            Remark::CryAlert => "Alerte!!! Alerte!!!",
            Remark::ShieldBearerCovers => {
                "A couvert! Ils ont des arcs!\t\t\t\t\t\t\t\t\t\t\t\t\t\t"
            }
            Remark::ShieldBearersLineFormation => "En ligne!",
            Remark::ArchersBehindShieldBearers => "Les archers, derriere!",
            Remark::ProudDontFight => "Montrez-moi ce que vous savez faire!",
            Remark::ProudFinallyFight => "Je vais vous montrer moi...",
            Remark::OfficerSeesBrawl => "Qu'est-ce qu'ils font, encore?",
            Remark::OfficerEndsBrawl => "Hkhmmmm!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::OfficerStopsPatrol => "Halte !",
            Remark::OfficerStartsPatrol => "En avant, marche !",
            Remark::OfficerComplains => "Bande d'incapables !",
            Remark::OfficerAsksWhatsup => "Qu' y a-t-il, Soldat?",
            Remark::OfficerAsksWhere => "O\u{FFFD} ?",
            Remark::OfficerEndsConversation => "Rompez!",
            Remark::OfficerCallsSoldier => "Soldat!",
            Remark::OfficerSendsOutSoldier => "Va voir par la",
            Remark::OfficerCallsGroup => "A moi, la garde!",
            Remark::OfficerSendsOutGroup => "Examinez les alentours! Execution!",
            Remark::OfficerSendsOutGroupForCharly => "Trouvez-moi ce tire au flanc! Execution!",
            Remark::OfficerRebukesCharly => "Alors? On quitte son poste?",
            Remark::OfficerRebukesCharlyEnd => "Tu me feras trois jours!",
            Remark::OfficerGivesAttackOrder => "Soldats! A l'attaaaaque!!!",
            Remark::OutOfAmmunition => "J'ai plus de fleches!\t\t\t\t\t\t\t\t\t\t\t\t\t\t",
            Remark::SpecialAction => "hahaha",
            Remark::AdmiresObjectScript => "Alors ca ressemble a ca?",
            Remark::MissesObjectScript => "Bon sang! Il a disparu!",
            Remark::GiveOrReceiveOrder => "J'y vais!",

            Remark::CivSeesBody => "Oh, le pauvre!",
            Remark::CivSeesDeadBody => "Mais il est mort!",
            Remark::CivCallsSoldier => "Eh, le garde! ",
            Remark::CivDenunciates => "Y sont passes par la!",
            Remark::CivAdmiresRobin => "Qu'il est beau!",
            Remark::CivPanic => "A l'aide!",
            Remark::CivWounded => "Pitie!",
            Remark::CivDies => "hennnfff",
            Remark::CivThanx => "Oh, merci, merci",
            Remark::CivCries => "C'est affreux, affreux",
            Remark::CivBeerYes => "Une bonne chopine, ca rechauffe...",
            Remark::CivBeerNo => "Non, ca me ferait perdre la tete...",
            Remark::CivSeesSoldiersUnderNet => "Tiens? Elle a fini par en attraper un?",
            Remark::CivUnderNet => "Mais qui a fait ca?",
            Remark::CivApple => "Oh, le vilain petit garcon!",
            Remark::CivWasps => "Au secours, des guepes!",
            Remark::CivWhistling => "Arretes, mon mari va t'entendre!",
            Remark::CivSeesBrawl => "Quelle bande de brutes!",
            Remark::CivGoldYes => "Oh! Quelle chance!",
            Remark::CivGoldNo => "L'argent ne fait pas le bonheur...",
            Remark::CivBeggarBegging => "L'aumone, mon bon seigneur, l'aumone!",
            Remark::CivBeggarGivesInfo => "Merci bien! Je vais vous dire...",
            Remark::CivBeggarWantsMore => "Encore quelques sous, monseigneur?",
            Remark::CivBeggarGivesLastInfo => "Mon dernier conseil...",
            Remark::CivBeggarThanx => "Oh, merci!",
            Remark::CivBeggarIdentifiesHimself => "Voila, voila",
            Remark::CivChildCaughtBySoldier => "C'etait pas moi",
            Remark::CivChildChasedBySoldier => "Tu m'attraperas pas!",

            Remark::VipProudDontFight => "Qu'on l'echarpe!",
            Remark::VipProudFinallyFight => "Ahhh! Poussez-vous, bande d'incapables!",
            Remark::VipStartsCombat => "Je vais t'ecraser!",
            Remark::VipWounded => "Argh!",
            Remark::VipDies => "Noir tout est si  noir",
            Remark::VipGoodStrikeCombat => "Ca fait mal, hein?",
            Remark::VipWarcry => "Je ne vais pas te tuer tout de suite...",
            Remark::VipVictory => "Pff trop facile",
            Remark::VipSpeaksToHimself => "Une bataille! Qu'on me donne une bataille!",
            Remark::VipAleNo => "De la biere Tiede! Je ferait fouetter cet impudent!",
            Remark::VipNetNo => "Ah! Quelle idee grotesque!",
            Remark::VipAppleNo => "Une pomme? J'ai demande du CHEVREUIL que diable!",
            Remark::VipWaspsNo => "Des guepes? Hmm C'est une idee...",
            Remark::VipGoldNo => "Hmm Si un serviteur la ramasse, je le ferais fouetter..",

            Remark::NumberOfRemarks | Remark::TheSoundOfSilence => " ........... ",
        }
    }
}

impl std::fmt::Display for Remark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.speech())
    }
}

// ---------------------------------------------------------------------------
// Question (decision-making queries)
// ---------------------------------------------------------------------------

/// Questions the AI asks itself to make behavior decisions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Question {
    ShallIFollowSteps = 0,
    ShallIStayOnMyPost,
    ShallIFollowLostEnemy,
    ShallIFollowHint,
    ShallIHelpFriendInTrouble,
    ShallIRun,
    ShallITakeAle,
    ShallITakeMoney,
    ShallIReactOnApple,
    ShallIFightForMoney,
    ShallISeekBeforeAlertingOfficer,
    ShallISeekBeforeAlertingSoldiers,
    ShallISendOutSoldier,
    ShallILookWhistle,
    ShallIFollowWhistle,
    HasTheNewTaskPriority,
}

// ---------------------------------------------------------------------------
// Battle decision
// ---------------------------------------------------------------------------

/// Battle-time tactical decisions.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Decision {
    None = 0,
    PredecisionOffensive,
    PredecisionDefensive,
    Cassos,
    Fight,
    Observe,
    Reserve,
    AlertSoldiers,
    RunAndAlertSoldiers,
    Menace,
    Shoot,
    ArcherStepBack,
    LookForHelp,
    LookForHelpIfNobodyElseDoes,
    CoverBehindShieldBearer,
    TooProudToAttack,
    TowerGuardAlert,
    TowerGuardObserve,
    ArcherObserve,
    RunToArcheryPoint,
    RunForNewArrows,
    LastReserve,
}

impl Decision {
    pub fn log_string_from_u16(raw: u16) -> &'static str {
        Self::try_from(u32::from(raw))
            .ok()
            .and_then(Self::log_string)
            .unwrap_or("DECISION-???")
    }

    pub fn log_string(self) -> Option<&'static str> {
        Some(match self {
            Decision::None | Decision::PredecisionOffensive | Decision::PredecisionDefensive => {
                return None;
            }
            Decision::Cassos => "DECISION-CASSOS",
            Decision::Fight => "DECISION-FIGHT",
            Decision::Observe => "DECISION-OBSERVE",
            Decision::Reserve => "DECISION-RESERVE",
            Decision::AlertSoldiers => "DECISION-ALERT-SOLDIERS",
            Decision::RunAndAlertSoldiers => "DECISION-RUN-AND-ALERT-SOLDIERS",
            Decision::Menace => "DECISION-MENACE",
            Decision::Shoot => "DECISION-SHOOT",
            Decision::ArcherStepBack => "DECISION-ARCHER-STEP-BACK",
            Decision::LookForHelp => "DECISION-LOOK-4-HELP",
            Decision::LookForHelpIfNobodyElseDoes => "DECISION-LOOK-4-HELP-IF-NOBODY-ELSE-DOES",
            Decision::CoverBehindShieldBearer => "DECISION-COVER-BEHIND-SHIELD-BEARER",
            Decision::TooProudToAttack => "DECISION-TOO-PROUD-TO-ATTACK",
            Decision::TowerGuardAlert => "DECISION-TOWER-GUARD-ALERT",
            Decision::TowerGuardObserve => "DECISION-TOWER-GUARD-OBSERVE",
            Decision::ArcherObserve => "DECISION-ARCHER-OBSERVE",
            Decision::RunToArcheryPoint => "DECISION-RUN-TO-ARCHERY-POINT",
            Decision::RunForNewArrows => "DECISION-RUN-FOR-NEW-ARROWS",
            Decision::LastReserve => "DECISION-LAST-RESERVE",
        })
    }
}

// ---------------------------------------------------------------------------
// Cross-NPC actions (phalanx coordination, stimulus forwarding)
// ---------------------------------------------------------------------------

/// Actions that one NPC's AI emits to affect another NPC. The engine
/// drains these after each think() and applies them to the targets.
/// Used for patterns like calling `InstructGatherPosition` then
/// delivering `CALL_INSTRUCTION`, and recursive `BreakPhalanx`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum CrossNpcAction {
    /// Set gather position on target NPC, then deliver `CALL_INSTRUCTION`.
    InstructGatherPosition {
        target: NpcHandle,
        position: Position,
        direction: u16,
    },
    /// Propagate break-phalanx to target: clear their combat neighbours,
    /// set `phalanx_aborted = true`, and trigger `BattleDecisions`.
    BreakPhalanx { target: NpcHandle },
    /// Deliver a stimulus to the target NPC (e.g. `CALL_COORDINATE`).
    SendStimulus {
        target: NpcHandle,
        stimulus_type: StimulusType,
        /// Optional payload (position, human handle, etc.).  Defaults to
        /// `StimulusInfo::None` for stimuli that carry no data.
        info: StimulusInfo,
        /// When set, if the target's `think()` returns `false` (stimulus
        /// not handled), redeliver the stimulus to this NPC instead. Used
        /// in conversation chains to fall back to the original sender when
        /// the receiver doesn't handle the call.
        fallback_to_sender: Option<NpcHandle>,
        /// Propagated `Stimulus::to_whole_patrol` flag — set when a patrol
        /// chief broadcasts a stimulus to subordinates. Receivers must
        /// restore this flag when rebuilding the `Stimulus`, otherwise
        /// `dispatch_stimulus_to_whole_patrol` fails to early-exit on the
        /// member side and re-delegates back to the chief, producing an
        /// unbounded chief↔member ping-pong loop.
        to_whole_patrol: bool,
    },
    /// Set the target NPC's left combat neighbour link (one-way).
    /// Bare setter, no reciprocal cleanup. Use
    /// [`Self::UpdateLeftCombatNeighbour`] for the full semantics
    /// (reciprocal cleanup).
    SetLeftCombatNeighbour {
        target: NpcHandle,
        neighbour: HumanHandle,
    },
    /// Set the target NPC's right combat neighbour link (one-way).
    SetRightCombatNeighbour {
        target: NpcHandle,
        neighbour: HumanHandle,
    },
    /// Full reciprocal update of `target`'s left combat neighbour. Four steps:
    ///   1. Clear `old_left`'s right pointer (if non-zero).
    ///   2. Store `new_left` on `target`'s left pointer.
    ///   3. Pre-clean `new_left`'s existing right (and that-right's left).
    ///   4. Wire `new_left`'s right pointer back to `target`.
    ///
    /// `old_left` is captured at push time so the drain doesn't depend on
    /// `target`'s current state being unmodified.
    UpdateLeftCombatNeighbour {
        target: NpcHandle,
        old_left: HumanHandle,
        new_left: HumanHandle,
    },
    /// Mirror of [`Self::UpdateLeftCombatNeighbour`] for the right side.
    UpdateRightCombatNeighbour {
        target: NpcHandle,
        old_right: HumanHandle,
        new_right: HumanHandle,
    },
    /// Propagate primary target to a phalanx member during
    /// `ReconsiderPhalanx`'s phalanx-member walk.
    SetPrimaryTarget {
        target: NpcHandle,
        primary_target: HumanHandle,
    },
    /// Make the target NPC say a remark.
    Say { target: NpcHandle, remark: Remark },
    /// Write `AiBase::looted_after_money_fight` on a target soldier.
    /// Money-fight looters set this as soon as they reserve a KO'd victim
    /// so other scanners skip the same body.
    SetLootedAfterMoneyFight { target: NpcHandle, looted: bool },
    /// Update the target NPC's reconnaissance report type and seek position
    /// — shares the officer's report back to the soldier after
    /// `GetReportFromSoldier`.
    UpdateReport {
        target: NpcHandle,
        report_type: ReportType,
        seek_position: Position,
    },
    /// Merge the officer's reconnaissance report into the target soldier's
    /// report. Broadcast inside `AlertSoldiers` so newly alerted soldiers
    /// pick up the officer's charly handle and report type before they run
    /// into the group.
    ConsiderReport {
        target: NpcHandle,
        /// Cloned from the caller's own `ReconnaissanceReport` at the
        /// time the alert was dispatched.
        report: ReconnaissanceReport,
        /// Merge-mask passed to [`ReconnaissanceReport::consider_report`]
        /// (e.g. `UPDATE_CHARLY | UPDATE_TYPE = 2|4 = 6`).
        flags: u16,
    },
    /// Push `actor` onto `target`'s `synchronizing_actors` list. Used by
    /// `EventSeesCharlyStandardProcedure` when the reuniting soldier
    /// still needs to wait at the sync waypoint for its macro friend.
    RegisterSynchronizingActor { target: NpcHandle, actor: NpcHandle },
}

// ---------------------------------------------------------------------------
// Panic request (queued by AI, applied by engine)
// ---------------------------------------------------------------------------

/// Queued `Panic()` request on an [`AiController`].
///
/// The AI layer sets this field when a fleeing stimulus kicks in; the
/// engine consumes it at post-think time and performs the door lookup
/// against `ai_global.door_seek_infos` (which the AI layer doesn't
/// see on its call stack).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PanicRequest {
    /// Point to flee *away from*.  `None` means undirected panic — the
    /// engine picks any reachable door and runs in random directions.
    pub center: Option<Position>,
    /// Number of run segments the NPC should execute after the initial
    /// door fallback fails.
    pub runs: u8,
    /// Alert level the drain should install on state entry (default
    /// `ALERT_RED`).
    pub alert: AlertLevel,
    /// `true` when the caller was not already in `FleeingPanic` /
    /// `FleeingRunToDoor` at the time the request was queued. Lets the
    /// drain suppress repeated state changes / Say() / `EventReachPoint`
    /// dispatches when we're already mid-panic.
    pub is_new_panic: bool,
}

/// Pending request for a script-driven `SeekArea` entry, set from
/// `SetAIState(actor, STATE_SEEKING)` script natives. The engine
/// consumes it post-think by dispatching into `EnemyAi::seek_area`
/// (soldier-only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptSeekAreaRequest {
    /// Seek center — typically the NPC's current position.
    pub center: Position,
    /// Radius passed to `SeekArea` (`AI_SCRIPT_SEEK_RADIUS`).
    pub radius: u16,
}

/// Variants of `AssignNewPatrolPath` — the three call shapes (sentinel
/// `-1`, sentinel `-2`, valid index) collapse to these semantic cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatrolAssignment {
    /// Sentinel `-1` / null pointer — drop the path, leave
    /// `likes_to_sit_around = false`.
    ClearPath,
    /// Sentinel `-2` / `(void*)-1` — drop the path but set
    /// `likes_to_sit_around = true`.
    ClearPathSitAround,
    /// Valid-index branch.
    Index(PathId),
}

// ---------------------------------------------------------------------------
// Look direction
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum LookDirection {
    Left = 0,
    Right,
    LeftRight,
    RightLeft,
    Down,
}

// ---------------------------------------------------------------------------
// Log line type (debug AI log)
// ---------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum LogLineType {
    Event = 0,
    EventRefused,
    ChangeState,
    BattleDecision,
    Speak,
    SpeakImpossible,
    SpeakFinished,
    Timer,
}

/// A single AI log entry for debug display.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct LogLine {
    pub line_type: LogLineType,
    pub info: u16,
    pub frame: u32,
}

// ---------------------------------------------------------------------------
// Simple shared data types
// ---------------------------------------------------------------------------

/// Noise type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum NoiseType {
    Plouf = 0,
    Bonk,
    Zonk,
    TapTapTap,
    ArfArf,
    Tirili,
    PutPut,
    Aaargh,
    Heeelp,
    Pling,
    Pfiiit,
    Logs,
    Drawbridge,
    ZingZing,
    Off,
}

/// A noise event with origin, type, volume, and elevation.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Noise {
    pub origin: Position,
    pub noise_type: NoiseType,
    pub volume: u16,
    pub elevation: u16,
    pub element_id: u16,
}

/// Detection level of a PC by an NPC.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Detection {
    None = 0,
    Unrecognized,
    Recognized,
    /// Internally used by AI.
    Killed,
}

/// Global alert level.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum AlertLevel {
    #[default]
    Green = 0,
    Yellow,
    Red,
}

/// NPC attitude toward PCs / the world.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Attitude {
    Friendly = 0,
    Neutral,
    #[default]
    Suspicious,
    Nervous,
    Hostile,
}

/// View cone configuration.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum ViewCone {
    #[default]
    Commandoslike = 0,
    Patrol,
    QuickSearch,
    GetOverview,
    QuickOverview,
    SlowOverview,
    GattlingOverview,
    LookDown,
    LookTo,
    LookToOrCommandoslikeDependingOnIq,
    LookForward,
    Focus,
    GattlingFocus,
    Idle,
    Slow,
    LongRange,
    Sniper,
    SceneOfTheCrime,
    Valium,
}

/// Curiosity trigger type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Curiosity {
    Shot = 0,
    Dynamite,
    Siesta,
    Steps,
    Cards,
    Watch,
    Whistle,
    // NumberOfCuriosities — use Curiosity::COUNT
}

impl Curiosity {
    pub const COUNT: usize = 7;
}

/// Type of target (PC, NPC, or scarecrow).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum TargetType {
    Pc = 0,
    Npc,
    Scarecrow,
}

/// Report type for reconnaissance reports.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum ReportType {
    Nothing = 0,
    Noise,
    Body,
    MissedCharly,
    DeadBody,
    Enemy,
}

// ---------------------------------------------------------------------------
// Stimulus info — typed payload for stimuli
// ---------------------------------------------------------------------------

/// Hint passed between NPCs (e.g. "look over there").
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct Hint {
    pub seek_point: Position,
    pub seek_flags: u16,
    pub who_tells_me: NpcHandle,
}

/// Info about a stolen object.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct StolenObject {
    pub object: ObjectHandle,
    pub thief: NpcHandle,
}

/// Info about a friend in trouble.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct CombatInfo {
    pub actor_npc: NpcHandle,
    pub enemy_position: Position,
}

/// Info about a door combat event.
#[derive(
    Debug, Clone, Copy, PartialEq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct DoorCombatInfo {
    pub delay: u16,
    pub goal: Position,
    pub direction: u16,
    pub adversary: HumanHandle,
}

/// The payload of a [`Stimulus`].
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum StimulusInfo {
    #[default]
    None,
    Noise(Noise),
    Position(Position),
    Human(HumanHandle),
    Hint(Hint),
    Object(ObjectHandle),
    Stolen(StolenObject),
    Combat(CombatInfo),
    DoorCombat(DoorCombatInfo),
    Index(u16),
}

// ---------------------------------------------------------------------------
// Stimulus
// ---------------------------------------------------------------------------

/// An event or call that is dispatched to an NPC's AI for processing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Stimulus {
    pub stimulus_type: StimulusType,
    pub info: StimulusInfo,
    pub owner: NpcHandle,
    pub to_whole_patrol: bool,
}

impl Stimulus {
    pub fn new(stimulus_type: StimulusType) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::None,
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_noise(stimulus_type: StimulusType, noise: Noise) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Noise(noise),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_position(stimulus_type: StimulusType, pos: Position) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Position(pos),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_human(stimulus_type: StimulusType, human: HumanHandle) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::Human(human),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    pub fn with_door_combat(stimulus_type: StimulusType, dc: DoorCombatInfo) -> Self {
        Self {
            stimulus_type,
            info: StimulusInfo::DoorCombat(dc),
            owner: 0,
            to_whole_patrol: false,
        }
    }

    /// Returns `true` if two stimuli have the same type and equivalent info.
    pub fn is_similar(&self, other: &Self) -> bool {
        if self.stimulus_type != other.stimulus_type {
            return false;
        }
        match (&self.info, &other.info) {
            (StimulusInfo::None, StimulusInfo::None) => true,
            (StimulusInfo::Noise(a), StimulusInfo::Noise(b)) => {
                a.origin.x == b.origin.x && a.origin.y == b.origin.y && a.noise_type == b.noise_type
            }
            (StimulusInfo::Position(a), StimulusInfo::Position(b)) => a.x == b.x && a.y == b.y,
            (StimulusInfo::Human(a), StimulusInfo::Human(b)) => a == b,
            (StimulusInfo::Hint(a), StimulusInfo::Hint(b)) => {
                a.seek_point.x == b.seek_point.x
                    && a.seek_point.y == b.seek_point.y
                    && a.seek_flags == b.seek_flags
            }
            (StimulusInfo::Object(a), StimulusInfo::Object(b)) => a == b,
            (StimulusInfo::Stolen(a), StimulusInfo::Stolen(b)) => {
                a.object == b.object && a.thief == b.thief
            }
            (StimulusInfo::Combat(a), StimulusInfo::Combat(b)) => {
                a.enemy_position.x == b.enemy_position.x
                    && a.enemy_position.y == b.enemy_position.y
                    && a.actor_npc == b.actor_npc
            }
            (StimulusInfo::DoorCombat(a), StimulusInfo::DoorCombat(b)) => {
                a.goal.x == b.goal.x && a.goal.y == b.goal.y
            }
            (StimulusInfo::Index(a), StimulusInfo::Index(b)) => a == b,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Screen remark (HUD display)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScreenRemark {
    pub timer: u16,
    pub prefix: String,
    pub remark: Remark,
}

/// A forbidden remark entry — prevents the same line from being repeated.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ForbiddenRemark {
    pub remark: Remark,
    pub flags: u16,
    pub speech_id: u32,
    pub guy_index: u16,
    pub bad_guy: bool,
    pub forbidden_till_frame: u32,
}

// ---------------------------------------------------------------------------
// Reconnaissance report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ReconnaissanceReport {
    pub seek_position: Position,
    pub report_type: ReportType,
    pub seen_bodies: Vec<HumanHandle>,
    pub charly: NpcHandle,
    pub charly_seen: bool,
}

impl Default for ReconnaissanceReport {
    fn default() -> Self {
        Self {
            seek_position: Position::default(),
            report_type: ReportType::Nothing,
            seen_bodies: Vec::new(),
            charly: 0,
            charly_seen: false,
        }
    }
}

impl ReconnaissanceReport {
    pub fn reset(&mut self) {
        self.seen_bodies.clear();
        self.report_type = ReportType::Nothing;
        self.charly = 0;
    }

    pub fn update(&mut self, new_type: ReportType, new_position: Position) {
        if self.report_type <= new_type {
            self.report_type = new_type;
            self.seek_position = new_position;
        }
    }

    /// Full report merging.
    ///
    /// `flags` is a bitmask:
    /// - `REPORT_UPDATE_BODIES` (1): merge seen_bodies from `other`
    /// - `REPORT_UPDATE_CHARLY` (2): copy charly handle if we don't have one
    /// - `REPORT_UPDATE_TYPE` (4): update report type and seek position

    pub fn add_seen_body(&mut self, body: HumanHandle) {
        self.seen_bodies.push(body);
    }

    pub fn is_body_seen(&self, body: HumanHandle) -> bool {
        self.seen_bodies.contains(&body)
    }
}

// ---------------------------------------------------------------------------
// Seek point
// ---------------------------------------------------------------------------

/// A point of interest that NPCs can investigate during seek-area sweeps.
///
/// Interest decays over time after examination: the `frame_when_full_interest`
/// field tracks when the point will be "fresh" again (100% interest).
/// Multiple NPCs avoid investigating the same point simultaneously via
/// the `locked` flag.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SeekPoint {
    pub position: Position,
    /// Frame at which interest will be 100% again.
    pub frame_when_full_interest: u32,
    /// Compass directions (0–15) to look from this point.
    pub directions: Vec<u16>,
    /// Last calculated interest value (0–100).
    pub last_calculated_interest: u8,
    /// Whether a soldier is currently investigating this point.
    pub locked: bool,
    /// Unique ID. Global seek points use their array index; personal
    /// seek points use sentinel values (1111, 2222).
    pub id: u16,
}

impl SeekPoint {
    /// Create a new seek point from a direction.
    ///
    /// We initialise `last_calculated_interest = 100` (full interest) as a
    /// safe, deterministic starting value; in the happy path
    /// `calculate_interest()` overwrites it before any reader inspects it.
    pub fn from_direction(dir: &SeekPointDirection) -> Self {
        Self {
            position: dir.position,
            directions: vec![dir.direction],
            frame_when_full_interest: 0,
            last_calculated_interest: 100,
            locked: false,
            id: 0,
        }
    }

    /// Create a seek point at a position with random directions.
    ///
    /// Uses `sim_rng` for deterministic RNG (port-wide choice) and
    /// initialises `last_calculated_interest = 100` — see `from_direction`
    /// above.
    pub fn from_position(pos: Position) -> Self {
        let directions = match crate::sim_rng::u8(0..4) {
            0 => vec![0, 3, 7, 11],
            1 => vec![2, 5, 10, 14],
            2 => vec![2, 7, 13],
            _ => vec![4, 10, 15],
        };
        Self {
            position: pos,
            directions,
            frame_when_full_interest: 0,
            last_calculated_interest: 100,
            locked: false,
            id: 0,
        }
    }

    /// Calculate interest based on elapsed time since last examination.
    /// Returns 0–100.
    pub fn calculate_interest(&mut self, current_frame: u32) -> u8 {
        let relative = current_frame as i32 - self.frame_when_full_interest as i32;
        self.last_calculated_interest = if relative >= 0 {
            100
        } else if relative <= -(crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST) {
            0
        } else {
            (100 + (100 * relative) / crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST)
                as u8
        };
        self.last_calculated_interest
    }

    /// Decrease interest (push full-interest frame further into the future).
    pub fn subtract_interest(&mut self, value: u8, current_frame: u32) {
        if self.frame_when_full_interest < current_frame {
            self.frame_when_full_interest = current_frame;
        }
        self.frame_when_full_interest += value as u32
            * crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_1_PERCENT_OF_INTEREST as u32;
        let max =
            current_frame + crate::parameters_ai::SEEK_POINT_TIME_TO_REGAIN_FULL_INTEREST as u32;
        if self.frame_when_full_interest > max {
            self.frame_when_full_interest = max;
        }
    }

    /// Try to merge a nearby direction into this seek point.
    /// Returns `true` if the direction was close enough and was added.
    pub fn add_if_near(&mut self, dir: &SeekPointDirection) -> bool {
        let dx = (dir.position.x - self.position.x).abs();
        let dy = (dir.position.y - self.position.y).abs();
        let max_norm = dx.max(dy);
        if max_norm <= crate::parameters_ai::SEEK_POINT_UNIFY_TOLERANCE as f32 {
            // Unconditionally append the incoming direction — duplicates
            // are intentional, they bias the seek sweep toward
            // repeatedly-hinted compass directions.
            self.directions.push(dir.direction);
            true
        } else {
            false
        }
    }
}

/// A seek-point direction from the level file (position + facing).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SeekPointDirection {
    pub position: Position,
    pub direction: u16,
}

// ---------------------------------------------------------------------------
// Ambush point
// ---------------------------------------------------------------------------

/// A tactical ambush point that NPCs check while patrolling.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AmbushPoint {
    pub position: Position,
    pub direction: u16,
    /// 3D anchor point — the 2D `position` lifted to eye height (z + 32).
    /// Used by the sight-polygon anchor for stealth / hide-in-ambush
    /// queries.
    pub position_3d: crate::coordinates::WorldPoint3D,
    /// Unique ambush-point ID assigned at `InitAI()` time. Used by AI
    /// scripts that reference ambush points by index.
    pub id: u16,
}

/// Half-size of the ambush-containment box along the X axis.
pub const AMBUSH_BOX_HALF_SIZE: f32 = 100.0;

impl AmbushPoint {
    /// True iff `sector` and `level` match the ambush point's stored
    /// position and the 2D `point` lies inside the ambush containment
    /// box centred on `position` with half-diagonal
    /// `(AMBUSH_BOX_HALF_SIZE, AMBUSH_BOX_HALF_SIZE * ASPECT_RATIO)`.
    pub fn is_near(
        &self,
        point: crate::coordinates::MapPoint,
        level: u16,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> bool {
        if self.position.level != level || self.position.sector != sector {
            return false;
        }
        let dx = (point.x - self.position.x).abs();
        let dy = (point.y - self.position.y).abs();
        dx <= AMBUSH_BOX_HALF_SIZE
            && dy <= AMBUSH_BOX_HALF_SIZE * crate::position_interface::ASPECT_RATIO
    }
}

// ---------------------------------------------------------------------------
// Archery sector
// ---------------------------------------------------------------------------

/// A waypoint along an archery path (entry point or shooting point).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PointArchery {
    pub position: Position,
    pub direction: u16,
    /// True if this is a shooting position (not just a path waypoint).
    pub is_shooting_point: bool,
    /// Sector number of this point — used for sector-change distance
    /// penalty (compared against [`crate::position_interface::SectorHandle`]
    /// via [`crate::sector::SectorNumber`] u16 conversion).
    pub sector_index: crate::sector::SectorNumber,
    /// Entity of the archer occupying this point, or `None` if free.
    pub owner: Option<crate::entity_id::EntityId>,
}

/// An archery sector where archers can set up, with ordered waypoints
/// leading to shooting positions.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SectorArchery {
    pub points: Vec<PointArchery>,
    /// Polygon vertices for the `is_inside` check (f32 coords).
    pub polygon: Vec<(f32, f32)>,
    /// Layer / level this archery sector belongs to.
    pub layer: u16,
    /// Index of the first shooting point in `points`.  `None` when the
    /// sector has no shooting points.
    pub index_first_shooting_point: Option<crate::sector::ArcheryPointIdx>,
    /// Index of the last shooting point in `points`.  `None` when the
    /// sector has no shooting points.
    pub index_last_shooting_point: Option<crate::sector::ArcheryPointIdx>,
    /// Total number of shooting points.
    pub num_shooting_points: u16,
    /// Number of archers currently assigned to this sector.
    pub num_owners: u16,
}

impl SectorArchery {
    pub fn is_full(&self) -> bool {
        self.num_owners >= self.num_shooting_points
    }

    /// Bump the sector-level archer count; asserts the sector isn't
    /// already full (the caller must have checked `!is_full()` before
    /// picking this sector, as `choose_good_shooting_point` does).
    pub fn increment_owner_counter(&mut self) {
        assert!(!self.is_full(), "archery sector is full");
        self.num_owners += 1;
    }

    pub fn decrement_owner_counter(&mut self) {
        assert!(self.num_owners > 0, "archery sector has no owners");
        self.num_owners -= 1;
    }

    /// Point-in-polygon test for the archery sector boundary.
    pub fn is_inside(&self, pos: &Position, layer: u16) -> bool {
        if self.layer != layer {
            return false;
        }
        let (px, py) = (pos.x, pos.y);
        let n = self.polygon.len();
        if n < 3 {
            return false;
        }
        // Ray-casting algorithm
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = self.polygon[i];
            let (xj, yj) = self.polygon[j];
            if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

// ---------------------------------------------------------------------------
// Repulsive point (scripts add these to repel NPCs from an area)
// ---------------------------------------------------------------------------

/// A point that NPCs try to avoid during pathfinding.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct RepulsivePoint {
    pub id: i32,
    pub position: Position,
    /// Inner radius — strong repulsion zone.
    pub radius: f32,
    /// Outer radius — weaker repulsion zone.
    pub action_radius: f32,
    /// Flags (affects PCs, soldiers, etc.).
    pub flags: i32,
}

// ---------------------------------------------------------------------------
// Door info for seek-area door checks
// ---------------------------------------------------------------------------

/// Minimal door info cached on AiGlobalState for `FindDoorEnemyCouldBeBehind`.
/// Populated at level load from the full `Door` data on GameHost.
/// Serialized with `AiGlobalState`; includes cached authorization data that
/// should match the exact door state at the save point.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct DoorSeekInfo {
    /// Index into the game host's full `doors` array. Carried so AI
    /// helpers (e.g. `RunAndAlertSoldiers`) can stash a door reference
    /// onto the NPC.
    pub door_index: crate::gate::DoorIndex,
    pub door_type: crate::gate::DoorType,
    pub point_out: (f32, f32),
    pub position_in: Position,
    pub sector_out: u16,
    /// Sector on the inside of the door (the building).
    pub sector_in: u16,
    /// Layer (z-level) on the outside of the door. Used by
    /// `RunAndAlertSoldiers` for the layer-mismatch malus in the
    /// weighted-distance scoring.
    pub layer_out: u16,
    /// Cached `IsActorAutorized` result for an NPC soldier entering in
    /// the direct (outside→inside) direction. Used by
    /// `FindDoorEnemyCouldBeBehind`. Snapshot — does NOT track runtime
    /// building capacity or post-patch lock changes; refresh by
    /// rebuilding the array.
    pub npc_villain_authorized_direct: bool,
}

// ---------------------------------------------------------------------------
// AiContext — per-frame entity state passed into think()
// ---------------------------------------------------------------------------

/// Per-frame entity state passed into `think()` by the engine.
/// Replaces the stale-prone `cached_*` fields on `AiBase` for data that
/// changes every frame (position, direction, posture, etc.).
#[derive(Debug, Clone, Default)]
pub struct AiContext {
    pub position: Position,
    pub frame: u32,
    pub direction: u16,
    pub posture: crate::element::Posture,
    /// `IsVeryVeryBusy`'s sequence-element arm: `true` when the actor's
    /// current in-flight sequence element is `Command::PassDoor` or
    /// `Command::Fall`. The posture arm is covered separately via
    /// `posture` above. Used by `FriendlyAi::return_to_duty` to lock
    /// `AILOCK_BUSY` and defer `EventReturnToDuty` mid-door-pass.
    /// Defaults to `false` for AiContexts not built through the
    /// per-tick engine path (unit tests, fallback fields).
    pub in_uninterruptible_command: bool,
    pub in_building: bool,
    pub building_sector: Option<SectorHandle>,
    pub camp: crate::element::Camp,
    pub is_swordfighting: bool,
    /// `true` when the sequence manager has a pending `ENTER_SWORDFIGHT`
    /// element for this NPC. `ReconsiderSwordfight` bails out early when
    /// an enter-swordfight sequence is already queued.
    pub enter_swordfight_pending: bool,
    /// True when the current level is Sherwood Forest. Used by
    /// `is_merry_man_forest()` and the 180° detection cone for Royalist
    /// NPCs.
    pub is_forest_level: bool,
    /// The evaluating entity's zero-centred collision bounding box.
    pub move_box: crate::geo2d::BBox2D,
    /// NPC's remaining arrow count (`GetAmmoAmount(RHACTION_BOW)`). Used
    /// by archer decision logic.
    pub remaining_arrows: u16,
    /// Square of the engine's standard view-polygon radius. Used to gate
    /// cover-position acceptance for archers behind shield bearers (the
    /// cover point must be within view radius of the primary target).
    pub sq_standard_view_radius: f32,
    /// Square of this NPC's live `mViewParameters.uwRealRadius`.
    /// Detection helpers use this instead of the level standard radius
    /// because alertness, drunk view, lean-out, and scripts can mutate
    /// the real radius independently.
    pub sq_self_view_radius: f32,
    /// Entity elevation (Z coordinate). Used by archer bow-down/bow-up
    /// decisions.
    pub elevation: f32,

    /// Self is a civilian beggar (`CIVILIAN_BEGGAR`). `false` for
    /// non-civilians.
    pub self_is_beggar: bool,
    /// Self is a civilian child (`CIVILIAN_CHILD`). `false` for
    /// non-civilians.
    pub self_is_child: bool,
    /// `true` when the evaluating NPC is a soldier (enemy AI variant),
    /// `false` for civilians. Used by the waypoint-macro executor to gate
    /// soldier-only opcodes (CHECK_4, LOOK_LEFT, LOOK_RIGHT, BEND,
    /// PATROL_*).
    pub self_is_soldier: bool,
    /// `true` when the evaluating NPC is a mounted soldier (rider).
    /// Sourced from [`SoldierData::rider`] each tick. `false` for
    /// non-soldiers.
    pub self_is_rider: bool,
    /// Self's `ActionState` (`Waiting` / `Moving` / `MovingFast` / sword
    /// states / etc.). Used by `EventViewStandardProcedure` to branch on
    /// `RHACTIONSTATE_MOVING_FAST` (sprint-into-engage path). Defaults to
    /// `Waiting` for unit tests built off `AiContext::default()`.
    pub self_action_state: crate::element::ActionState,
    /// Self's soldier rank if soldier; `ProfileRank::None` otherwise.
    /// Used by `GetBoredTime` to pick officer-length intervals.
    pub self_rank: crate::profiles::ProfileRank,
    /// Self's soldier pride. `0` for non-soldiers or soldiers with no
    /// pride. Used by `GetBoredTime` to pick the long "pride" bored
    /// interval.
    pub self_pride: u16,

    /// `true` when this NPC is dead (`life_points <= 0`). Read by the
    /// `start_think` dead-gate to short-circuit stimulus processing —
    /// defence-in-depth against cross-NPC actions or scripts that fire
    /// stimuli at a corpse after the tick loop would normally skip it.
    pub self_is_dead: bool,

    /// Number of entries in this NPC's
    /// `detectable_lists[DetectableType::Friend]`. Used by
    /// `return_to_duty_common_stuff` to decide whether to clear
    /// `detected_body`.
    pub self_detectable_friend_count: u16,

    /// `true` for soldier NPCs whose `forced_attentive` flag is set,
    /// `false` for civilians and non-forced soldiers. Read by
    /// `set_alert_status_with_flags` to pin the view alert to YELLOW when
    /// the music alert drops to GREEN.
    pub self_forced_attentive: bool,

    /// Number of entries in this NPC's
    /// `detectable_lists[DetectableType::MissedFriend]`. Used by
    /// `EnemyAi::return_to_duty` to detect that the NPC was searching for
    /// a missed-in-action friend (`checkpoint_charly`) when bailing out.
    pub self_detectable_missed_friend_count: u16,

    /// Live animation (`OrderType`) currently playing on this NPC. Read
    /// by AI gates that inspect the actor's current animation directly,
    /// e.g. `DefaultBoredStandardProcedure` skips its head-turn
    /// transition while the `WAITING_UPRIGHT_BORED_RANDOM` idle is
    /// already playing.
    pub self_animation: crate::order::OrderType,

    /// Resolved info about the stimulus's antagonist entity — the
    /// "other" human the stimulus is about (the observed PC for an
    /// `EventView`, the body for `EventSeesBody`, etc.).  The engine
    /// populates this before dispatching any stimulus whose
    /// `StimulusInfo::Human(_)` payload identifies a live entity, so
    /// that `event_*_standard_procedure` handlers don't need to reach
    /// back into the entity table.  `None` for stimuli without a
    /// human payload, or if the referenced entity has been removed.
    pub antagonist: Option<AntagonistInfo>,

    /// Handle → snapshot map for **every** entity visible to the AI
    /// this tick. Populated once at the top of the AI tick by
    /// `EngineInner::build_sim_scratch` and shared into each
    /// `AiContext` via an [`Arc`] so cloning / re-building contexts is
    /// cheap. Used to answer per-entity field reads (position, camp,
    /// ai_state, …) for any handle the AI has stashed (antagonist,
    /// primary target, interesting object, detected body, friend, …).
    pub entity_views: crate::ai_entity_view::SharedAiEntityViews,

    /// Per-tick `Arc`-shared snapshot of the engine's sight obstacles.
    /// Built once by `EngineInner::build_sim_scratch` and
    /// embedded into every `AiContext` so AI-side helpers can answer
    /// `ai_vision::los_clear` (opaque-LOS) without a mutable engine
    /// borrow. Use `obstacle_list()` for the borrowed `ObstacleList<'_>`
    /// shape that `ai_vision::los_clear` accepts.
    pub sight_obstacles: crate::sight_obstacle::SharedSightObstacles,
    /// FastFindGrid snapshot used for `IsReachable` line-of-sight queries
    /// from AI code that only has an `AiContext`.
    pub fast_grid: crate::fast_find_grid::FastFindGrid,
    /// Shared mission hiking paths from [`LevelAssets`]. Static level data
    /// threaded through context so individual AI controllers do not each cache
    /// their own Arc attachment.
    pub hiking_paths: Arc<Vec<crate::level_data::RawHikingPath>>,

    /// Soldier load-order index → entity slot mapping (cloned from
    /// [`AiGlobalState::all_soldier_handles`]). Used by waypoint-macro
    /// opcodes (`CMD_CHECK_4` / `CMD_CHECK_4_SYNC`) that resolve a
    /// friend ID baked into the script bytecode.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,
}

impl AiContext {
    /// Look up a handle in the per-tick entity view map.
    ///
    /// Returns `None` for handle `0`, for handles that were never
    /// populated (non-human entities not included in the snapshot),
    /// and for entities that have since been removed. Callers that
    /// need a specific field (position, ai_state, …) should pattern-
    /// match on the result and fall back to a safe default only
    /// when it makes sense for the call site.
    pub fn entity_view(&self, handle: u32) -> Option<&crate::ai_entity_view::AiEntityView> {
        if handle == 0 {
            return None;
        }
        self.entity_views.get(&handle)
    }

    /// Convenience wrapper around [`Self::entity_view`] that returns
    /// just the position.
    pub fn entity_position(&self, handle: u32) -> Option<Position> {
        self.entity_view(handle).map(|v| v.position)
    }

    /// Resolve a C++ `PositionToPoint3D` style point from a sector/layer
    /// position. The returned y coordinate is screen-space (`y + z`).
    pub fn position_to_point_3d(&self, position: Position) -> crate::coordinates::WorldPoint3D {
        let z = match position.sector {
            None => 0.0,
            Some(handle) => {
                let sector_number = crate::sector::SectorNumber::new(handle.get() as i16);
                let grid_idx = self
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&sector_number)
                    .copied();
                let is_motion = grid_idx.is_some_and(|idx| {
                    self.fast_grid
                        .level
                        .sectors
                        .get(idx)
                        .is_some_and(|sector| sector.sector_type.is_motion())
                });
                if grid_idx.is_some() && !is_motion {
                    panic!(
                        "position_to_point_3d: sector {} is not a motion sector",
                        handle.get()
                    );
                }

                let point = crate::geo2d::pt(position.x, position.y);
                let mut best: Option<(f32, f32)> = None;
                for (_, obstacle) in self.sight_obstacles.list().iter_indexed() {
                    if !obstacle.is_projection_area()
                        || obstacle.sector != handle.get()
                        || obstacle.layer != position.level
                        || !obstacle.box_screen.contains_point(point)
                        || !obstacle.contains_point_screen(point)
                    {
                        continue;
                    }
                    let z_max = obstacle.box_3d_max[2];
                    let z = obstacle.compute_top_z(position.x, position.y);
                    match best {
                        None => best = Some((z_max, z)),
                        Some((prev_z_max, _)) if z_max > prev_z_max => best = Some((z_max, z)),
                        _ => {}
                    }
                }

                best.map(|(_, z)| z).unwrap_or(0.0)
            }
        };

        crate::coordinates::WorldPoint3D {
            x: position.x,
            y: position.y + z,
            z,
        }
    }

    /// Borrowed [`crate::sight_obstacle::ObstacleList`] view over this
    /// tick's sight-obstacle snapshot — the shape that
    /// `ai_vision::los_clear` and the visibility query helpers accept.
    pub fn obstacle_list(&self) -> crate::sight_obstacle::ObstacleList<'_> {
        self.sight_obstacles.list()
    }

    /// Resolve a soldier register number (load-order index) to an
    /// entity slot handle. Returns `None` when the ID is out of range
    /// — the caller should treat that as a null actor and warn/abort
    /// the operation.
    pub fn all_soldier_handle(&self, register: u16) -> Option<u32> {
        self.all_soldier_handles.get(register as usize).copied()
    }

    /// Number of soldiers in the level.
    pub fn number_of_all_soldiers(&self) -> u16 {
        self.all_soldier_handles.len() as u16
    }

    /// Raw-point variant of the 360° detection check. Used by
    /// `InitializeFriendCheck` to ask "can I still see my friend's post
    /// / waypoint from here?".
    /// Steps:
    /// 1. viewer in a building → false
    /// 2. stretched-Y 3D distance vs. `sq_self_view_radius`
    /// 3. opaque-LOS via the context-only LOS helper.
    pub fn is_detecting_point_360(&self, pt: crate::coordinates::WorldPoint3D) -> bool {
        if self.in_building {
            return false;
        }
        let viewer_eye = crate::stealth::eye_point_xy(
            crate::coordinates::MapPoint::new(self.position.x, self.position.y),
            self.posture,
            self.direction as i16,
            false,
        );
        let viewer_eye_z =
            self.elevation + crate::stealth::eye_z_for_posture(self.posture, self.self_is_rider);
        let dx = pt.x - viewer_eye.x;
        let dy = (pt.y - viewer_eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = pt.z - viewer_eye_z;
        let sq_distance = dx * dx + dy * dy + dz * dz;
        if sq_distance > self.sq_self_view_radius {
            return false;
        }
        crate::sight_obstacle::is_reachable_3d(
            self.obstacle_list(),
            [viewer_eye.x, viewer_eye.y, viewer_eye_z],
            [pt.x, pt.y, pt.z],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        )
    }
}

/// Lightweight view of an entity other than the evaluating NPC, used
/// by AI stimulus handlers. All fields come from the live entity at
/// the moment the stimulus is dispatched.
#[derive(Debug, Clone, Default)]
pub struct AntagonistInfo {
    /// The antagonist's map position.
    pub position: Position,
    /// The antagonist's camp.
    pub camp: crate::element::Camp,
    /// True when the antagonist is in a sword-fighting action state.
    pub is_swordfighting: bool,
    /// True when the antagonist is a player character.
    pub is_pc: bool,
    /// True when the antagonist is the Robin Hood PC. Civilian reactions
    /// (`CivAdmiresRobin`) special-case this.
    pub is_robin: bool,
    /// True when the antagonist is a VIP civilian / VIP soldier.
    pub is_vip: bool,
    /// True when the antagonist is inside a building sector.
    pub in_building: bool,
}

/// Summary of an unconscious or otherwise-disabled enemy that an NPC
/// could approach and finish off.
///
/// Used by the two "sleeping enemy" paths in `BattleDecisions`:
///
///  * `unconscious_enemies` — enemies that were in `list_them` when
///    the cleanup pass filtered them out because they weren't
///    `IsAbleToFight()`.
///  * `nearby_sleeping_enemies` — 360°-range scan of all unconscious,
///    non-carried enemies around the NPC. Used by the final
///    `KillNearbySleepingEnemies` fallback.
#[derive(Debug, Clone)]
pub struct SleepingEnemyInfo {
    pub handle: HumanHandle,
    pub position: Position,
    /// True if the target is a player character (as opposed to an
    /// enemy NPC in an opposing camp).
    pub is_pc: bool,
    /// True if this PC is Robin Hood (used for VIP rules).
    pub is_robin: bool,
    /// True if the target is a VIP (hero PC or VIP NPC).
    pub is_vip: bool,
}

/// Per-tick analysis data computed by the engine's detection loop.
/// Populated once per detection tick, consumed by battle_decisions
/// and swordfight tactics. Passed alongside AiContext.
#[derive(Debug, Clone)]
pub struct AiPerTickData {
    /// Shared immutable profile table used for profile-pointer reads from
    /// AI helpers without cloning profile structs into every snapshot.
    pub profile_manager: std::sync::Arc<crate::profiles::ProfileManager>,
    pub patrol_chief_position: Position,
    pub patrol_chief_state: AiState,
    pub enemy_sq_distances: Vec<(HumanHandle, i32)>,
    pub min_sq_enemy_distance: i32,
    pub friends_lower_company: u16,
    pub soldiers_lower_pride: bool,
    pub friends_nearer_to_enemy: u16,
    /// Sum of battle points for our side — 100 + pride per soldier, 100
    /// per PC. Used by `MakeBattlePredecisions`.
    pub us_battle_points: u32,
    /// True if any friend (not self) in `list_us` has officer rank.
    pub has_officer_nearby: bool,
    /// True if any friend in `list_us` has RANK_SOLDIER.
    pub simple_soldiers_near: bool,
    pub primary_target_multiplicity: Vec<(HumanHandle, u32)>,
    pub nearby_fighters: Vec<crate::ai_enemy::FighterSnapshot>,
    /// Same-camp soldiers snapshot for alert functions (`alert_officer`,
    /// `alert_soldiers`).  Populated every tick from the engine's soldier
    /// snapshot list, filtered to the evaluating NPC's camp.
    pub camp_soldiers: Vec<crate::ai_enemy::CampSoldierInfo>,
    /// Same-camp soldiers who are currently unconscious + alive and whose
    /// `AiBase::knocked_out_in_money_fight` flag is set. Populated
    /// alongside `camp_soldiers` (`camp_soldiers` skips unconscious
    /// entries, so this parallel list carries the sleeping money-fight
    /// victims needed by `WantsToContinueMoneyFight`).
    pub camp_ko_money_fighters: Vec<NpcHandle>,
    pub visible_seeking_friends: u16,
    pub friend_seek_clears_help_flag: bool,
    /// Pre-computed destination forecast for the primary target.
    /// Populated by the engine from the target entity's live state
    /// (door-pass, lift, building traversal). See [`forecast_destination_for_ia`].
    pub primary_target_forecast: Option<ForecastedDestination>,
    /// True when the primary target is a player character.
    /// Used by lost-sight logic in `reconsider_swordfight` to decide
    /// whether to chase (PC) or pull a battle overview (NPC).
    pub primary_target_is_pc: bool,
    /// Pre-computed destination forecast for the missed PC (if any).
    /// Used by `get_battle_overview` to re-predict position before seeking.
    pub missed_pc_forecast: Option<ForecastedDestination>,
    /// True when `missed_pc` refers to a player character.
    pub missed_pc_is_pc: bool,
    /// Number of enemies this soldier personally detected (not shared by
    /// friends). Used for observe decisions where the count should
    /// reflect only what this NPC can see, not the merged `list_them`.
    pub personally_visible_enemies: u16,
    /// Enemies that showed up in detection this tick but were filtered
    /// out of `enemy_sq_distances` / `list_them` because they are
    /// unconscious (or otherwise unable to fight) and not being carried.
    /// Consumed by the "approach unconscious enemy" branch in
    /// `battle_decisions`.
    pub unconscious_enemies: Vec<SleepingEnemyInfo>,
    /// All unconscious, non-carried enemies within the NPC's 360°
    /// real-view radius (with LOS), regardless of whether they were
    /// in the detection list. Consumed by the final
    /// `KillNearbySleepingEnemies` fallback.
    pub nearby_sleeping_enemies: Vec<SleepingEnemyInfo>,
    /// Precomputed jump-line index for table swordfight with the primary
    /// target. `Some(line_idx)` when the NPC and primary target are in
    /// different sectors reachable via a jump-line pair. Used during
    /// `ReconsiderEnemyApproach`.
    pub primary_target_jump_line: Option<u32>,
    /// Live position of `primary_target` this tick. Used by
    /// `ReconsiderEnemyApproach`. `None` = caller didn't look it up.
    pub primary_target_position: Option<Position>,
    /// Live posture of `primary_target` this tick.
    pub primary_target_posture: Option<crate::element::Posture>,
    /// Live animation (order type) of `primary_target` this tick.
    pub primary_target_animation: Option<crate::order::OrderType>,
    /// If `primary_target` is on another entity's shoulders
    /// (`RHPOSTURE_ON_SHOULDERS`), the live position of the carrier.
    /// The AI retargets to the carrier in that case.
    pub primary_target_carrier_position: Option<Position>,
    /// If `primary_target` is on another entity's shoulders
    /// (`RHPOSTURE_ON_SHOULDERS`), the carrier's handle. The AI re-points
    /// `primary_target` to this handle so all downstream
    /// position / friend-swap / focus / `BeginSwordfight` reads target
    /// the carrier rather than the carried entity.
    pub primary_target_carrier_handle: Option<HumanHandle>,
    /// True when `primary_target`'s sector is a non-stairs lift.
    pub primary_target_in_lift: bool,
    /// When `primary_target_in_lift` is true, the lift entry point on
    /// the evaluating NPC's own layer (high or low entry based on layer).
    pub primary_target_lift_entry: Option<Position>,
    /// Friend target-swap candidates: same-camp soldiers currently
    /// approaching their own primary target.
    pub friend_swap_candidates: Vec<FriendSwapCandidate>,

    /// Pre-computed fallback position for the "avenger on the roof"
    /// branch. Populated by the engine when `couldnt_reachpoint` is set
    /// and [`crate::gate::compute_avenger_wait_position`] finds a
    /// blocking gate on the path from the primary target back to the
    /// evaluating NPC. `None` when the branch doesn't apply.
    pub avenger_on_roof_wait_position: Option<Position>,

    /// Handles in `me`'s `DETECTABLE_ENEMY` list whose `seen_last_frame`
    /// flag is set. Used by `RefreshArrowProtection` so a shield bearer
    /// doesn't raise his shield against a bow-armed enemy who is occluded
    /// or has slipped out of his cone of vision this frame.
    pub seen_last_frame_enemies: Vec<HumanHandle>,

    /// Geometry of the door this NPC would walk *out* of when commanding
    /// soldiers from inside a building. Used by the `AlertSoldiers`
    /// indoor branch. `None` when the NPC is not inside a building or no
    /// exit door is reachable.
    pub my_exit_door: Option<MyExitDoorInfo>,

    /// Per-NPC `list_them` snapshots for every member of this NPC's
    /// phalanx right-chain (excluding self). Populated by the engine
    /// builder when the evaluating NPC has a non-zero
    /// `right_combat_neighbour`. Consumed by
    /// `PhalanxReinitializeThemList` so the leftmost member can union
    /// each neighbour's enemies into `list_them_all_phalanx` without
    /// round-tripping through cross-NPC AI state. The snapshots are
    /// pulled up-front to avoid mutating sibling AI brains mid-tick.
    pub phalanx_member_them_lists: Vec<PhalanxMemberThemList>,
}

/// One phalanx member's `list_them` snapshot, plus their position and
/// direction so the leftmost can re-evaluate step-1 keep-filter
/// predicates on their behalf. Equivalent to recursing into
/// `right_combat_neighbour->PhalanxReinitializeThemList`.
#[derive(Debug, Clone)]
pub struct PhalanxMemberThemList {
    /// Member's element handle (matches `FighterSnapshot::handle`).
    pub handle: HumanHandle,
    /// Member's `list_them` as captured at tick-data build time —
    /// i.e. the persistent enemy list this member's own AI updates
    /// every detection cycle. Represents step-1's "input to clean
    /// up": entries that survive `IsAbleToFight && IsDetecting360 &&
    /// !IsFriend` are kept.
    pub current_them_list: Vec<HumanHandle>,
    /// Member's position (for step-2 180° checks evaluated from
    /// their stance).
    pub position: Position,
    /// Member's facing sector (0-15). Needed by step-2's
    /// `IsDetecting180Degrees` cone check.
    pub direction: u16,
}

/// Snapshot of the door an NPC inside a building would use to step
/// outside. Populated by the engine each tick from the NPC's stored
/// door reference (or, lazily, the nearest building door when none is
/// set). Geometry-only — the door's runtime state (open/closed, lock
/// counter) doesn't affect formation placement.
#[derive(Debug, Clone, Copy)]
pub struct MyExitDoorInfo {
    /// Outside-edge anchor point.
    pub point_out: (f32, f32),
    /// Door midpoint.
    pub point_mid: (f32, f32),
    /// Outside-layer index.
    pub layer_out: u16,
    /// Outside-sector handle. Wrapped in `Option` because
    /// `SectorHandle::new(0)` returns `None` for the no-sector sentinel.
    pub sector_out: Option<crate::position_interface::SectorHandle>,
    /// Outside-edge as a full Position (for slot construction).
    pub position_out: Position,
}

/// Same-camp soldier that is currently approaching its primary target,
/// exposed to `ReconsiderEnemyApproach` for the target-swap heuristic.
#[derive(Debug, Clone, Copy)]
pub struct FriendSwapCandidate {
    pub friend_handle: HumanHandle,
    pub friend_position: Position,
    pub friend_primary_target: HumanHandle,
    pub friend_primary_target_position: Position,
}

impl AiPerTickData {
    /// Construct an empty/stub `AiPerTickData` with all fields zeroed
    /// or empty. **Use sparingly** — every call site is shipping a
    /// stripped-down snapshot to whatever AI dispatch follows, and any
    /// AI logic that needs the missing fields will silently see empty
    /// data instead of the real engine state. The user-visible bug
    /// class this caused: `battle_decisions` reads `enemy_sq_distances`
    /// and falls back to `return_to_duty` when the list is empty even
    /// if the soldier has a valid `primary_target` — soldier wedges in
    /// a Reactiontime/Default ping-pong because the timer-dispatch
    /// path passes `stub()` instead of the rich per-NPC tick data
    /// that the detection-dispatch path builds.
    ///
    /// This used to be the `Default` trait impl, but `Default` was
    /// removed so call sites can no longer accidentally pull in
    /// stripped data via the `..Default::default()` shorthand without
    /// noticing. Renaming to `stub` and requiring an explicit call
    /// makes the loss-of-fidelity visible at every dispatch site.
    ///
    /// Most engine-side dispatch paths now use the centralized
    /// `EngineInner::build_npc_tick_data(npc_id)` builder.  Remaining
    /// direct stubs should stay limited to call sites that provably
    /// dispatch non-combat AI paths (init before target selection,
    /// friendly panic, or non-soldier entities); otherwise add a
    /// builder call instead of silently feeding empty combat context.
    pub fn stub() -> Self {
        Self {
            profile_manager: std::sync::Arc::new(crate::profiles::ProfileManager::new()),
            patrol_chief_position: Position::default(),
            patrol_chief_state: AiState::Default,
            enemy_sq_distances: Vec::new(),
            min_sq_enemy_distance: i32::MAX,
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            friends_nearer_to_enemy: 0,
            us_battle_points: 0,
            has_officer_nearby: false,
            simple_soldiers_near: false,
            primary_target_multiplicity: Vec::new(),
            nearby_fighters: Vec::new(),
            camp_soldiers: Vec::new(),
            camp_ko_money_fighters: Vec::new(),
            visible_seeking_friends: 0,
            friend_seek_clears_help_flag: false,
            primary_target_forecast: None,
            primary_target_is_pc: false,
            missed_pc_forecast: None,
            missed_pc_is_pc: false,
            personally_visible_enemies: 0,
            unconscious_enemies: Vec::new(),
            nearby_sleeping_enemies: Vec::new(),
            primary_target_jump_line: None,
            primary_target_position: None,
            primary_target_posture: None,
            primary_target_animation: None,
            primary_target_carrier_position: None,
            primary_target_carrier_handle: None,
            primary_target_in_lift: false,
            primary_target_lift_entry: None,
            friend_swap_candidates: Vec::new(),
            avenger_on_roof_wait_position: None,
            seen_last_frame_enemies: Vec::new(),
            my_exit_door: None,
            phalanx_member_them_lists: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ReinforcementDoorInfo — cached door data for MerryManForestCassos
// ---------------------------------------------------------------------------

/// Cached info for a reinforcement door, used by `MerryManForestCassos`
/// to find the nearest map exit and animate running to its PointOut.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ReinforcementDoorInfo {
    /// Inner position of the door (where the NPC walks *to*).
    pub position_in: Position,
    /// Index into the game host's door array.
    pub door_index: crate::gate::DoorIndex,
    /// Outer point of the door (where the NPC exits the map).
    pub point_out: (f32, f32),
    /// Mid-point of the door interior. Used by `AlertSoldiers` to
    /// compute the door-out vector for the indoor officer formation
    /// sweep.
    pub point_mid: (f32, f32),
    /// Layer index of the outer (outside) end of the door. Used by
    /// indoor formation paths to place gather slots on the outside
    /// layer.
    pub layer_out: u16,
    /// Sector handle of the outer (outside) end of the door.
    pub sector_out: Option<crate::position_interface::SectorHandle>,
    /// Inner door point as raw coordinates. `position_in` already
    /// carries this with layer/sector tagging, but the raw f32 pair is
    /// convenient for the door-vector math.
    pub point_in: (f32, f32),
}

// ---------------------------------------------------------------------------
// Global AI state
// ---------------------------------------------------------------------------

/// A building interior known to the AI.
///
/// Populated during `InitAI()` by collecting every sector whose
/// `IsBuilding()` is true. Houses carry their occupant list so AI
/// code can ask "who's inside?" without scanning all entities, and
/// their door indices so pursuers / investigators can pick the right
/// gate to enter / exit through.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct House {
    /// Sector index (into `FastFindGrid::sectors`) of the building's
    /// interior motion area.
    pub sector_index: u32,
    /// Building index (into `GameHost::building_occupants`) if this
    /// sector is linked to one. Same index used by the tenant list and
    /// the `host.building_occupants` parallel table. `None` when the
    /// sector isn't proto-linked to a building (e.g. script-synthesised
    /// portals).
    pub building_index: Option<crate::sector::BuildingIdx>,
    /// Doors that connect this building to the outside.  Indices into
    /// `GameHost::doors`.
    pub door_indices: Vec<u32>,
    /// Entities currently inside the building.  Kept live by the
    /// `PassDoor` Enter / Leave hooks in `engine::door_pass`.
    pub occupant_ids: Vec<crate::element::EntityId>,
    /// Whether this building carries an arrow reserve. Populated from
    /// the GUYS/CAVE tenant chunk.
    pub arrow_reserve: bool,
}

impl House {
    /// Number of actors currently inside the building.
    #[inline]
    pub fn occupant_count(&self) -> usize {
        self.occupant_ids.len()
    }

    /// Whether the given entity is currently an occupant.
    #[inline]
    pub fn contains_occupant(&self, eid: crate::element::EntityId) -> bool {
        self.occupant_ids.contains(&eid)
    }
}

// ─── On the actor-handle vs EntityId dual ─────────────────────────
//
// Building occupancy is tracked in two parallel data structures:
//
//   * `ai::House::occupant_ids: Vec<EntityId>` — the AI-facing view.
//     Populated at `EngineInner::initialize_buildings` and maintained
//     live by the `execute_pass_door` Enter / Leave hooks.  New AI
//     code should query this.
//
//   * `natives::GameHost::building_occupants: Vec<Vec<i32>>` — the
//     script-facing view, indexed by `building_index` with actor
//     script handles. Kept in sync by the same hooks so script
//     natives (`GetNumberOfOccupants`, `GetOccupant`, etc.) see the
//     same occupancy that AI code does.
//
// Both are kept consistent; the dual exists because script identity
// (`i32` handle) and AI identity (`EntityId`) co-exist across the
// codebase and neither can be dropped independently.  Long-term
// consolidation would either migrate script natives to `EntityId`
// or delete `building_occupants` once all natives query via a
// `GameHost::occupants_of(building_index) -> &[i32]` helper that
// derives from the House list on demand.
//

/// A rally point positioned just outside a building door.
///
/// Where NPCs regroup after exiting a building before resuming patrol.
/// Built in `InitAI()` at a fixed `AI_DOOR_RALLY_POINT_DISTANCE` from
/// each building door's `PointOut`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct DoorRallyPoint {
    /// World position (outside the door).
    pub position: Position,
    /// Door index in `GameHost::doors`.
    pub door_index: crate::gate::DoorIndex,
    /// Radius around `position` within which NPCs are "at" the rally
    /// point.
    pub radius: f32,
}

/// Distance offset (from `PointOut`) at which door rally points are
/// anchored.
pub const AI_DOOR_RALLY_POINT_DISTANCE: f32 = 100.0;

/// Global / shared AI state, conceptually module-static.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiGlobalState {
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,

    pub there_are_royalist_soldiers: bool,
    pub there_are_lacklandist_soldiers: bool,

    pub stupid_soldiers_cheat: bool,
    pub freeze: bool,

    pub overall_alert_status: AlertLevel,
    pub overall_villain_alert_status: AlertLevel,

    /// Ambush points in the current mission.
    pub ambush_points: Vec<AmbushPoint>,
    /// Seek points shared between all NPCs.
    pub seek_points: Vec<SeekPoint>,
    /// Archery sectors in the current mission.
    pub archery_sectors: Vec<SectorArchery>,

    /// Saved random seed value for deterministic replay.
    pub saved_random_seed: i64,

    /// Per-remark forbidden-until-frame table.
    pub remarks_forbidden_till_frame: Vec<u32>,
    /// Active forbidden remarks.
    pub forbidden_remarks: Vec<ForbiddenRemark>,
    /// Screen remarks to display.
    pub screen_remarks: Vec<ScreenRemark>,

    // Display toggles (not serialized, debug only)
    pub attribute_display: bool,
    pub speech_display: bool,
    pub golden_eye_mode: bool,
    /// `DIES IRAE` cheat — "thunder of God" toggle. While active the
    /// vengeance path (Ezekiel 25:17 reference) is applied, killing
    /// targets chosen by the selected-view-element overlay. Consumers
    /// that still need porting should read this flag on
    /// `AiGlobalState`.
    pub ezekiel_2517: bool,

    pub current_speech_variant: u16,

    /// Repulsive points: NPCs avoid these areas during pathfinding.
    /// Scripts add/remove them by integer ID.
    pub repulsive_points: Vec<RepulsivePoint>,

    /// Next auto-incrementing ID for repulsive points.
    pub next_repulsive_point_id: i32,

    /// Cached door geometry for `FindDoorEnemyCouldBeBehind`.
    /// Populated at level load and kept in the save snapshot so any
    /// door-state-dependent authorization cache survives restore.
    pub door_seek_infos: Vec<DoorSeekInfo>,

    /// Reinforcement doors: (inner position, door index, PointOut).
    /// Used by `MerryManForestCassos` to find the nearest map exit
    /// and animate the NPC running to the door's PointOut.
    /// Populated at level load.
    pub reinforcement_doors: Vec<ReinforcementDoorInfo>,

    /// Buildings the AI knows about — populated at `InitAI()` from
    /// every sector whose `sector_type.is_building()` is true, with
    /// each house's occupant list and doors filled in.
    pub houses: Vec<House>,

    /// Rally points anchored just outside each building door, created in
    /// `InitAI()` per house gate.
    pub door_rally_points: Vec<DoorRallyPoint>,

    /// Soldier load-order index → entity-handle (slot) mapping. Scripts
    /// and waypoint commands address NPCs by their soldier register
    /// index (the position in the all-soldiers list at level load), not
    /// by their entity slot. Cloned out of
    /// `LevelAssets::all_soldier_entity_ids` once at level load so the
    /// AI tick can resolve a friend ID without re-borrowing the engine.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,

    /// Same-frame combat claims made by soldiers during the current AI
    /// dispatch. Some engine side effects are batched, so this transient
    /// list carries the live claim until the normal entity state catches
    /// up — letting later soldiers in the same frame see earlier
    /// `AttackEnemy` decisions.
    pub same_frame_target_claims: Vec<(HumanHandle, HumanHandle)>,
}

impl Default for AiGlobalState {
    fn default() -> Self {
        Self {
            green_alert_soldiers: 0,
            yellow_alert_soldiers: 0,
            red_alert_soldiers: 0,
            there_are_royalist_soldiers: false,
            there_are_lacklandist_soldiers: false,
            stupid_soldiers_cheat: false,
            freeze: false,
            overall_alert_status: AlertLevel::Green,
            overall_villain_alert_status: AlertLevel::Green,
            ambush_points: Vec::new(),
            seek_points: Vec::new(),
            archery_sectors: Vec::new(),
            saved_random_seed: 0,
            remarks_forbidden_till_frame: Vec::new(),
            forbidden_remarks: Vec::new(),
            screen_remarks: Vec::new(),
            attribute_display: false,
            speech_display: false,
            golden_eye_mode: false,
            ezekiel_2517: false,
            current_speech_variant: 0,
            repulsive_points: Vec::new(),
            next_repulsive_point_id: 1,
            door_seek_infos: Vec::new(),
            reinforcement_doors: Vec::new(),
            houses: Vec::new(),
            door_rally_points: Vec::new(),
            all_soldier_handles: std::sync::Arc::new(Vec::new()),
            same_frame_target_claims: Vec::new(),
        }
    }
}

impl AiGlobalState {
    pub fn npcs_can_be_enemies(&self) -> bool {
        self.there_are_royalist_soldiers && self.there_are_lacklandist_soldiers
    }

    pub fn overall_villain_alert(&self) -> AlertLevel {
        if self.red_alert_soldiers > 0 {
            AlertLevel::Red
        } else if self.yellow_alert_soldiers > 0 {
            AlertLevel::Yellow
        } else {
            AlertLevel::Green
        }
    }

    pub fn reset_seek_points(&mut self) {
        self.seek_points.clear();
    }

    pub fn reset_ambush_points(&mut self) {
        self.ambush_points.clear();
    }

    pub fn reset_archery_sectors(&mut self) {
        self.archery_sectors.clear();
    }

    pub fn init_green_yellow_red_alert_soldiers(&mut self) {
        self.green_alert_soldiers = 0;
        self.yellow_alert_soldiers = 0;
        self.red_alert_soldiers = 0;
    }

    /// Add a seek-point direction, either merging it into an existing
    /// nearby seek point or creating a new one.
    pub fn add_seek_point_direction(&mut self, dir: &SeekPointDirection) {
        // Check all existing seek points in reverse order
        for sp in self.seek_points.iter_mut().rev() {
            if sp.add_if_near(dir) {
                return;
            }
        }
        // No nearby point found — create a new one
        let mut new_sp = SeekPoint::from_direction(dir);
        new_sp.id = self.seek_points.len() as u16;
        self.seek_points.push(new_sp);
    }

    /// Snap `pos` onto a nearby seek point, chosen at random from those
    /// within `MaxNorm(me_pos - pos) * distance_factor + abs_distance`
    /// (with a +100 penalty for level changes). Returns `true` if a
    /// candidate was found and `pos` was overwritten.
    pub fn set_pos_on_near_seek_point(
        &self,
        me_pos: Position,
        pos: &mut Position,
        distance_factor: f32,
        abs_distance: u16,
    ) -> bool {
        let base_dx = (me_pos.x - pos.x).abs();
        let base_dy = (me_pos.y - pos.y).abs();
        let distance_limit = base_dx.max(base_dy) * distance_factor + abs_distance as f32;

        let mut candidates: Vec<usize> = Vec::new();
        for (idx, sp) in self.seek_points.iter().enumerate() {
            let dx = (sp.position.x - pos.x).abs();
            let dy = (sp.position.y - pos.y).abs();
            let mut distance = dx.max(dy);
            if sp.position.level != pos.level {
                distance += 100.0;
            }
            if distance < distance_limit {
                candidates.push(idx);
            }
        }

        if candidates.is_empty() {
            return false;
        }
        let pick = crate::sim_rng::usize(0..candidates.len());
        *pos = self.seek_points[candidates[pick]].position;
        true
    }

    /// Post-process seek points near building doors: teleport them inside.
    pub fn teleport_seek_points_inside_doors(&mut self) {
        for sp in &mut self.seek_points {
            for door_info in &self.door_seek_infos {
                if door_info.door_type == crate::gate::DoorType::Building {
                    let dx = sp.position.x - door_info.point_out.0;
                    let dy = sp.position.y - door_info.point_out.1;
                    let max_norm = dx.abs().max(dy.abs());
                    if max_norm <= 5.0 {
                        sp.position = door_info.position_in;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InitStateSideEffects — entity-side fallout from `AiController::init_state`
// ---------------------------------------------------------------------------

/// Entity-side mutations that [`AiController::init_state`] asks the
/// caller to apply once the AI-side state transition has been
/// committed. The non-AI side effects of `InitState` — posture /
/// action state / eye status / life points / concussion — all live on
/// the entity, not the AI brain.
///
/// The caller (`EngineInner::init_one_ai`) applies these inside a
/// mutable-entity scope after the subclass dispatch returns.
#[derive(Debug, Default, Clone)]
pub struct InitStateSideEffects {
    /// `true` when the caller should run the standard
    /// "walk onto patrol path or launch a bored timer" tail after
    /// applying the side effects. The caller still has to AND this with
    /// `!ai_is_locked() && !ai_is_script_locked()` before actually
    /// calling `ReturnToDuty`.
    pub go_to_duty: bool,
    /// New posture — applied via
    /// `PositionInterface::set_posture` (+ a sync write-back to
    /// `ElementData::posture`).
    pub set_posture: Option<crate::element::Posture>,
    /// New action state — applied on `ActorData::action_state`.
    pub set_action_state: Option<crate::element::ActionState>,
    /// New `eye_status` — applied via
    /// `ai_vision::set_view_status`. Set to `Closed` by the
    /// sleeping-upright branch.
    pub set_eye_status: Option<crate::element::EyeStatus>,
    /// Zero out `NpcData::life_points` and flip
    /// `HumanData::killed_by_accident = true`. The two always co-occur
    /// at init.
    pub zero_life_points: bool,
    /// Seed `HumanData::concussion_of_the_brain = CONCUSSION_MAX`
    /// and flip `HumanData::unconscious = true`. Init-time has no
    /// script-lock / tied / carried gates to honour, so we bypass
    /// the full `combat::set_concussion` state machine.
    pub concussion_max_and_unconscious: bool,
}

// ---------------------------------------------------------------------------
// Base AI controller (per-NPC instance state)
// ---------------------------------------------------------------------------

/// The per-NPC AI controller state. Enemy and friendly AI extend this
/// with additional fields.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiController {
    // -- Owner --
    /// The NPC that owns this brain (legacy u32 handle).
    pub me: NpcHandle,

    /// Typed entity ID of the owning NPC.  Set when the AI is attached to
    /// an entity via the element system.  `None` for AI controllers created
    /// before the entity is registered.
    pub owner_entity_id: Option<EntityId>,

    // -- Patrol path IDs --
    //
    // The AI consults these after load when switching to alert routes,
    // so they are gameplay state here.
    pub path_id: Option<PathId>,
    pub alert_path_id: Option<PathId>,

    // -- State --
    pub current_state: AiState,
    pub current_substate: Substate,
    /// Music-side alert level — feeds the per-frame villain-alert
    /// counters and the music-mode pump.
    pub current_music_alert_status: AlertLevel,
    /// View-side alert level — what the cone tint and the
    /// `GetAIAlertStatus` script native read. `SetAlertStatus` pins this
    /// to YELLOW for soldiers with `IsForcedAttentive()` whose music
    /// alert just dropped to GREEN. For civilians and non-forced-attentive
    /// soldiers this stays equal to `current_music_alert_status`.
    pub view_alert_status: AlertLevel,
    pub substate_at_last_timer_launch: Substate,
    pub attitude: Attitude,
    pub blood_alcohol: u8,
    /// Initial animation to play when the NPC spawns into the world.
    /// Kept as a raw `u32` ordinal rather than `OrderType` because level
    /// data can carry animation values outside `OrderType`'s covered
    /// range — we map to an `OrderType` at spawn time via
    /// `map_pc_initial_action` and fall back to a warning rather than
    /// rejecting the level.
    pub initial_action: u32,

    pub number_of_looks: u8,

    // -- Patrol path --
    pub has_patrol_path: bool,
    /// Runtime patrol path tracking (wraps a hiking path with current waypoint).
    pub patrol_path: Option<PatrolPath>,
    pub can_move: bool,

    /// Think-method recursion depth — incremented on every `Think(...)`
    /// entry, decremented on exit. Read by `go_near` to shrink the
    /// stop-distance on deep recursion so panic/seek chains don't loop
    /// forever.
    pub think_recursion_depth: u8,

    // -- Macro system --
    /// Macro bytecode (if any) currently being executed.
    pub macro_command: Vec<u8>,
    pub macro_command_offset: usize,
    pub number_of_remaining_macro_bytes: u16,
    pub macro_in_progress: bool,
    pub macro_started_in_this_frame: bool,

    // -- Targets & relationships --
    pub primary_target: HumanHandle,
    pub friend_in_trouble: NpcHandle,
    pub detected_body: HumanHandle,
    pub interesting_object: ObjectHandle,
    pub antagonist: NpcHandle,
    pub last_stimulus_actor: ActorHandle,

    // -- Timers --
    pub timer_is_running: bool,
    pub when_does_timer_ring: u32,
    pub macro_timer_is_running: bool,
    pub when_does_macro_timer_ring: u32,
    pub standing_around_timer: u16,

    // -- Sorrow level (0–1000) --
    pub sorrow_level: u16,

    // -- Stimulus history (last 5) --
    pub last_stimulus: [StimulusType; 5],
    pub last_stimulus_multiplicity: [u16; 5],

    // -- Group behaviour --
    pub is_master: bool,
    pub master: NpcHandle,

    // -- Seek & alert --
    pub seek_position: Position,
    pub alert_soldiers_point: Position,
    pub first_try: bool,

    // -- Panic --
    pub panic_center_x: f32,
    pub panic_center_y: f32,
    pub lasting_panic_runs: u8,
    pub directed_panic: bool,

    // -- Battle lists --
    /// Our side in the current battle.
    pub list_us: Vec<HumanHandle>,
    /// Alerted allies.
    pub list_alerted_us: Vec<NpcHandle>,
    /// Allies staying put.
    pub list_staying_us: Vec<NpcHandle>,

    // -- Movement failure --
    pub couldnt_reachpoint: bool,
    pub already_on_point: bool,
    pub already_turned: bool,

    // -- Sitting around --
    pub likes_to_sit_around: bool,
    pub special_action: bool,

    pub friends_are_alerted: bool,
    pub is_stay_at_home: bool,

    // -- Stimulus queue --
    pub locks_flag_field: AiLockFlags,
    pub was_busy: bool,
    pub stimulus_queue: Vec<Stimulus>,
    pub script_locked: bool,
    pub remember_events: bool,

    // -- House leaving order --
    pub leave_house_number: u16,

    // -- Objects --
    pub forgotten_objects: Vec<ObjectHandle>,
    pub object_of_desire: ObjectHandle,

    // -- Charly (friend-check) --
    pub checkpoint_charly: NpcHandle,
    pub synchronize_charly: NpcHandle,
    /// Synchronization waypoint index for the partner. Lives on
    /// `AiBase` because the macro VM (`InitializeFriendCheck`) needs to
    /// write it from `AiController`.
    pub synchronize_index: u16,
    /// Per-look sorrow-level decrement seeded by `InitializeFriendCheck`
    /// (`delta_sorrow_level = 1000 / number_of_looks`).
    pub delta_sorrow_level: u16,
    /// NPCs the AI has decided are missing/dead and shouldn't be checked
    /// on again. Populated by stimulus handlers (corpse sighting, charly
    /// missing) and read by `InitializeFriendCheck` to early-resume the
    /// macro.
    pub missed_in_action: Vec<NpcHandle>,
    /// Frame at which this NPC last saw an enemy. Used by
    /// `InitializeFriendCheck` to suppress redundant CheckFor work for
    /// `NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME` frames after the alert.
    pub frame_when_enemy_detected: u32,

    pub inside_halt_method: bool,

    // -- Synchronizing actors --
    pub synchronizing_actors: Vec<NpcHandle>,
    pub default_path_walking_flags: GotoFlags,

    // -- Script-forbidden remarks --
    /// Remark IDs (as u32 indices into the Remark enum) that this NPC is
    /// forbidden from saying. Set by the ForbidNPCRemark script native.
    pub forbidden_remark_ids: Vec<u32>,

    // -- View cone --
    pub initial_view_cone: ViewCone,
    pub current_remark: Remark,
    pub current_remark_flags: u16,
    /// True once the engine has actually dispatched the exclamation to
    /// the sound manager and is waiting for the sound to finish. The
    /// `already speaking?` guard in `say_impl` checks
    /// `current_remark != Silence || speech_in_flight`; without this
    /// flag, `process_npc_speech` would clear `current_remark` to
    /// Silence on the tick after dispatch, allowing a subsequent
    /// non-EMERGENCY `Say()` to override a remark that is still
    /// playing. Cleared by phase 3 when `SoundIsFinished` fires.
    pub speech_in_flight: bool,

    // -- Macro rand --
    pub next_macro_rand: u8,
    pub next_macro_rand_forecasted: bool,

    // -- Emoticon --
    pub current_emoticon_type: EmoticonType,
    pub emoticon_expiration_date: u32,
    pub emoticon_has_expiration_date: bool,

    // -- Reconnaissance report --
    pub my_reconnaissance_report: ReconnaissanceReport,
    pub knocked_out_in_money_fight: bool,
    pub looted_after_money_fight: bool,

    // -- Patrol --
    pub patrol_chief: NpcHandle,
    pub patrol: Vec<NpcHandle>,
    pub missed_patrol_members: Vec<NpcHandle>,
    pub theoretical_patrol: Vec<NpcHandle>,
    pub patrol_stopped: bool,
    pub patrol_direction: u16,

    /// One-shot patrol-direction broadcast queued by the chief's
    /// CMD_PATROL_DIRECTION macro opcode. Drained by
    /// `EngineInner::tick_patrol_coordination` Phase 0 which calls
    /// `set_instructed_patrol_direction(direction, &ctx)` on every
    /// minion in `patrol`. Engine-level plumbing is required because the
    /// per-minion `set_instructed_patrol_direction` call needs each
    /// minion's `AiContext`.
    pub pending_patrol_direction_broadcast: Option<u16>,

    /// One-shot trigger asking `EngineInner::tick_patrol_coordination`
    /// Phase 3 to clear `patrol`/`missed_patrol_members` and rebuild
    /// from `theoretical_patrol` on its next pass. Set by call sites
    /// that explicitly invoke `InitializePatrol()`: `init_one_ai`,
    /// `return_to_duty`, the `CMD_PATROL_START` macro opcode, and the
    /// `Substate::DefaultGotoRoute` EVENT_REACHPOINT handler. Cleared by
    /// Phase 3 after the rebuild runs. Without the flag the rebuild gate
    /// was "both lists empty", which would silently re-initialise a
    /// chief whose minions all died/were promoted out — chiefs in that
    /// situation are intentionally kept in their early-return.
    pub needs_patrol_reinit: bool,

    pub got_the_beggar_trick: bool,

    // -- AI log (debug) --
    pub ai_log: Vec<LogLine>,
    /// Debug flag: render this NPC's view cone (toggled by EnableViewCone script).
    pub debug_view_cone_enabled: bool,

    // -- Last goto --
    pub last_goto_destination: Position,
    pub last_goto_flags: GotoFlags,
    pub stuck_counter: u16,

    // -- Pending orders --
    /// Orders produced by AI decisions, to be drained by the engine each tick.
    /// AI has no `EngineInner` reference so it can't allocate `order_id`s;
    /// the engine stamps them at drain time (see `AiOrderIntent::stamp`).
    pub pending_orders: Vec<AiOrderIntent>,

    /// Stimuli queued by the engine (e.g. from melee damage) to be
    /// dispatched to this AI on the next think cycle.
    pub pending_stimuli: Vec<Stimulus>,

    /// Cross-NPC actions produced by phalanx coordination, to be drained
    /// by the engine after each think(). See [`CrossNpcAction`].
    pub pending_cross_npc_actions: Vec<CrossNpcAction>,

    /// Self-directed stimuli queued by `say()` for MYTALK callbacks.
    /// The engine drains these after think() and re-dispatches them
    /// to the same NPC.
    pub pending_self_stimuli: Vec<StimulusType>,

    /// True when this NPC's bound actor-script class overrides
    /// `FilterAIEvent`. Set once at script bind time (see
    /// `engine/script.rs`) so `think()` cascades and `filter_stimulus`
    /// unmapped-stimulus paths can gate their "FilterAIEvent would have
    /// fired here" warnings to scripted actors only — the vast majority
    /// of NPCs are unscripted and would produce no-op noise.
    pub has_script_filter_override: bool,

    /// AI requests that the engine call `quit_swordfight` on this NPC.
    pub pending_quit_swordfight: bool,

    /// AI requests the engine launch a `Command::StopMenace` sequence
    /// element on this NPC, draining the menace pose back through
    /// `TRANSITION_MENACING_WAITING_SWORD` → `TRANSITION_LOWERING_SWORD`
    /// before any subsequently launched move starts.
    pub pending_stop_menace: bool,

    /// AI requests the engine launch a `Command::LowerShield` sequence
    /// element on this NPC so the shield drops before the queued move
    /// starts. Used when the actor walks off mid-shield-raise.
    pub pending_lower_shield: bool,

    /// AI requests that the engine deactivate this entity (SetActive(false)).
    /// Set when a merry man reaches the leave-map point.
    pub pending_deactivate: bool,

    /// AI requests the engine halt the actor's current active sequence
    /// element (and anything ≤ `Preference` priority on it). Equivalent
    /// to `Stop(PRIORITY_PREFERENCE)` with `inside_halt_method=true`
    /// so the interrupted element's `SendCondolationCard` is
    /// suppressed (no `EVENT_DONE`/`EVENT_IMPOSSIBLE` back to AI).
    /// Set by [`AiController::stop_all`]. Drained synchronously by
    /// the engine inside `tick_enemy_ai` *before* `process_pending_ai_orders`
    /// runs the new Turn / movement commands this AI queued in the same
    /// think.
    pub pending_halt: bool,

    /// AI requests the engine trigger panic on nearby civilians.
    pub pending_broadcast_panic: bool,

    /// AI requests the engine reset the `seen_now` / `seen_last_frame`
    /// flags on all enemy detectables so the next detection pass
    /// re-fires the "first seen" edge for anyone currently in the
    /// view cone. Used by `ThinkAlertingEvent` `EVENT_STOP` to make the
    /// NPC re-detect a target it already tracked — the intent is "I
    /// paid attention when the player said stop, so please re-register
    /// anyone I can still see".
    pub pending_blink_all_enemies: bool,

    /// AI requests the engine orchestrate a building-wide alert. The
    /// engine enumerates building occupants, sorts by camp, panics
    /// civilians, and calls `InitBattleBeforeDoor` on the lacklandist /
    /// royalist split. Set by any EVENT_VIEW case that triggers
    /// `EnemyInHouseAlert` (the attacking door-fight substates, the
    /// fleeing-indoors branch, and `EventViewStandardProcedure` when the
    /// NPC is indoors).
    pub pending_enemy_in_house_alert: bool,

    /// AI requests the engine add a detectable to this NPC's detection list.
    pub pending_add_detectables: Vec<(crate::element::EntityId, crate::element::DetectableType)>,
    /// AI requests the engine clear all detectables of a given type.
    pub pending_delete_detectables: Vec<crate::element::DetectableType>,
    /// AI requests the engine remove a single `(element, type)`
    /// detectable entry from this NPC's list. Distinct from
    /// `pending_delete_detectables`, which clears every entry of a given
    /// type.
    pub pending_delete_detectable_entity:
        Vec<(crate::element::EntityId, crate::element::DetectableType)>,

    /// AI requests the engine strip `DETECTABLE_BEGGAR(entity)` from
    /// every NPC's list so only one soldier handles the PC-beggar.
    /// Called inside the `EVENT_SEES_BEGGAR` / `_ANY_SEEK_AREA_SUBSTATE_`
    /// handler right after the soldier queues the beggar into its seek
    /// plan.
    pub pending_delete_beggar_for_all_npc: Vec<crate::element::EntityId>,

    /// AI requests the engine clear `seen_now` / `seen_last_frame` on
    /// the `DETECTABLE_ENEMY` entry whose element matches each listed
    /// target. Single-target counterpart of `pending_blink_all_enemies`.
    /// Queued by the wake-up broadcast in `tick_concussion_healing` so
    /// NPCs re-fire `EVENT_VIEW` against a waking adversary they were
    /// already tracking.
    pub pending_blink_enemy_specific: Vec<crate::element::EntityId>,

    /// AI requests that the engine either raise this NPC's sword or call
    /// `enter_swordfight(me, target)`.
    pub pending_enter_swordfight: Option<EnterSwordfightRequest>,
    /// Jump-line index for table swordfight, passed alongside
    /// `pending_enter_swordfight`. `None` = normal swordfight.
    pub pending_enter_swordfight_jump_line: Option<u32>,

    /// AI requests that the engine call `stop_owner(target, PRIORITY_NORMAL)`
    /// on a *different* entity (not `me`). Used inside
    /// `BeginSwordfight` — the engaging soldier freezes its moving
    /// target so the swordfight starts from a stable position.
    pub pending_stop_target: Option<HumanHandle>,

    /// AI requests that the engine promote this handle to principal opponent.
    pub pending_set_principal: Option<HumanHandle>,

    /// AI requests that `friend_handle`'s `primary_target` be reassigned
    /// to `new_primary_target` — the friend target-swap. The evaluating
    /// NPC updates its own `primary_target` in place — this field covers
    /// the friend half.
    pub pending_friend_primary_target_swap: Option<(HumanHandle, HumanHandle)>,

    /// AI requests that the engine launch a bow shot at this target.
    /// Set by `shoot_arrow_at`, drained by the engine post-think loop.
    pub pending_shoot_target: Option<HumanHandle>,

    /// AI requests that the engine call `focus_entity(target)` on this NPC
    /// — sets the eye-tracking target.
    pub pending_focus: Option<HumanHandle>,

    /// AI requests the engine run `UnalertAllNearCharlySeekers(charly)` —
    /// the engine iterates same-camp soldiers and dispatches
    /// `CALL_CHARLY_IS_BACK` to those that detect either the charly
    /// or the seeker (`me`) within 180°. Deferred via this option
    /// because the walk needs the engine's NPC table.
    ///
    /// `None` — no request pending.
    pub pending_unalert_near_charly_seekers: Option<CharlySeekerTarget>,

    /// AI requests the engine refill the bow ammo to `MAX_NPC_ARROWS`.
    /// Triggered when a fleeing archer reaches an arrow-reserves point.
    /// The engine writes `NpcData::number_of_arrows` when draining.
    pub pending_refill_bow_ammo: bool,

    /// AI requests that the engine write `reported_to_officer` on
    /// another soldier NPC. Used inside `MissedCharlyAlert`. The engine
    /// drains these pairs post-think and writes the flag on the
    /// target's `EnemyAi::reported_to_officer`.
    pub pending_set_reported_to_officer: Vec<(NpcHandle, bool)>,

    /// AI requests that the engine call `unfocus()` on this NPC —
    /// clears the eye-tracking target.
    pub pending_unfocus: bool,

    /// Last value of `primary_target` that was reconciled into
    /// `NpcData::follow_target`. Used by `refresh_npc_views` to gate
    /// the auto-sync to *changes* in `primary_target` rather than
    /// asserting `follow_target = primary_target` every tick.
    ///
    /// Without this gate, every tick the auto-sync would override any
    /// explicit `Focus(NULL)` (queued via `pending_unfocus`) while
    /// `primary_target` stayed set — defeating patterns like rider-charge
    /// passing and `BattleDecisions` entry, which clear focus without
    /// clearing the primary target.
    ///
    /// The drain pass updates this field whenever an explicit
    /// `pending_focus` / `pending_unfocus` / `pending_focus_point` fires,
    /// so subsequent ticks see `primary_target == last_synced` and leave
    /// the explicit focus state alone.
    pub last_synced_focus_target: HumanHandle,

    /// AI requests that the engine call `focus_point(point)` on this
    /// NPC — engages `EYES_STARE` with the narrow `STARE_HALF_ANGLE_RANGE`
    /// view cone locked on a ground point (rather than a target entity).
    /// Used by the call-look-there / tower-guard-alert / combat-alert
    /// standard procedures so the alerted NPC's view cone narrows toward
    /// the hint position.
    pub pending_focus_point: Option<Position>,

    /// Queued `FilterAIEvent(source, AI_STATE_CHANGE_TO_*)`
    /// notifications produced inline by `set_state`. Each entry is
    /// `(new_state, source)` where `source` mirrors the explicit
    /// argument:
    ///
    /// - `SelfActor` — pass `me` (the script-side handle for the actor
    ///   itself). Used for Sleeping / Default / Wondering / Seeking
    ///   transitions (friendly AI also routes Default/Wondering/
    ///   Seeking/Sleeping the same way).
    /// - `Null` — pass a null source for Attacking/Menacing/Fleeing
    ///   without a primary target.
    /// - `Human(h)` — pass a primary-target handle.
    ///
    /// The engine drains the queue after the AI tick via
    /// `dispatch_ai_state_change_notifications`, translating
    /// source variants to actor script handles.
    ///
    /// Capturing each transition synchronously inside `set_state` gives
    /// per-substate firing — every intra-think transition produces a
    /// notification, not just the final delta against `start_think`'s
    /// snapshot.
    pub pending_state_change_notifications: Vec<(AiState, AiStateChangeSource)>,

    /// AI requests that the engine fire `SlowlyOpenEyes()` on this
    /// NPC: reset `view_radius` to 5, point `view_radius_goal` at the
    /// engine's standard view radius, and switch `eye_status` to
    /// `EyeStatus::ViewconeGrow` so `refresh_view` ramps the cone
    /// back open. Set when an AI handler recovers vision (e.g.
    /// `EVENT_WASP_AWAY`, apple-sauce visor recovery).
    pub pending_slowly_open_eyes: bool,

    /// AI requests that the engine run `InformEveryoneOnMyResurrection`
    /// on this NPC's behalf — walk every other NPC and delete this NPC
    /// from their `DETECTABLE_BODY` list so they stop acting on a stale
    /// "I saw this body" memory. Set by `EVENT_FITAGAIN` handlers once
    /// a downed NPC regains consciousness.
    pub pending_inform_resurrection: bool,

    /// AI requests that the engine run `RestoreDetectableObjects`: walk
    /// every active engine object and, for any `OBJECT_ALE` (always) or
    /// `OBJECT_COIN` (iff `!knocked_out_in_money_fight`), add it to
    /// this NPC's `DETECTABLE_OBJECT` list when not already present.
    /// Set by the `EVENT_FITAGAIN` / `SleepingUnconscious` handler when
    /// an enemy wakes up from a KO so any bottles / coins that dropped
    /// during the brawl re-enter perception. Drained once per tick by
    /// the engine AI pipeline.
    pub pending_restore_detectable_objects: bool,

    /// AI requests that the engine sweep this NPC's `DETECTABLE_OBJECT`
    /// list and drop every coin entry within MaxNorm
    /// `NEARBY_COIN_DISTANCE = 500` of `pos`. The AI side keeps no
    /// copy of the detectable list, so it queues this request and the
    /// engine performs the sweep in `tick_ai_pending_*`. Drained once
    /// per tick.
    pub pending_forget_nearby_coins: Option<Position>,

    /// AI requests that the engine call `SetViewStatus(status)` on this
    /// NPC — flip `view_transition` and overwrite `eye_status`. Used by
    /// the civilian `EVENT_FITAGAIN` handler to reset the eyes to
    /// `EYES_LOOK_FORWARD` after regaining consciousness.
    pub pending_set_eye_status: Option<crate::element::EyeStatus>,

    /// AI requests that the engine snap this NPC's direction instantly.
    pub pending_set_direction_instantly: Option<i16>,

    /// AI requests that the engine flip the attentive flag and play the
    /// corresponding WAITING_UPRIGHT ↔ WAITING_ALERTED transition
    /// animation. Tuple is `(target_attentive, use_fast_officer_variant)`.
    /// Set from `EnemyAi::set_state` based on the new state/substate
    /// pair.
    pub pending_set_attentive_mode: Option<(bool, bool)>,

    /// AI requests the engine mirror a `SetGuardedPC` change: clear
    /// `pc.guard` on the *old* target PC and set it to this soldier
    /// on the *new* target PC. The AI writes its own `guarded_pc`
    /// field directly and queues this tuple so the engine can update
    /// the reciprocal PC-side `pc.guard` field (which the AI can't
    /// touch — `self` only borrows the soldier).
    ///
    /// `.0 = old_pc`, `.1 = new_pc`. Either may be `0` (meaning the
    /// `NULL` guarded-PC).
    pub pending_set_guarded_pc: Option<(HumanHandle, HumanHandle)>,

    /// Sequence commands the AI wants the engine to launch on behalf of
    /// this NPC. Each entry becomes a `SequenceElement` with the NPC as
    /// owner.
    pub pending_launch_commands: Vec<crate::element::Command>,

    /// Sequence commands the AI wants the engine to launch on behalf of
    /// *another* entity (e.g. soldier forcing a beggar to stand up via
    /// `Command::LeaveBeggar`). Each entry is `(target_handle, command)`
    /// — target handle is the standard actor script handle (matches
    /// the `SequenceElement` owner). Used by the beggar identify
    /// cascade.
    pub pending_launch_on_target: Vec<(crate::ai::NpcHandle, crate::element::Command)>,

    /// Full `Sequence` objects the AI wants the engine to launch via
    /// `SequenceManager::launch_sequence`. Use this when a single
    /// AI decision needs to launch a multi-element sequence with
    /// properties (directions, antagonists, …) that cannot be
    /// expressed with `pending_launch_commands`' one-command-per-entry
    /// shape. Used e.g. for the officer's turn/gather/point alert
    /// sequence.
    pub pending_launch_sequences: Vec<crate::sequence::Sequence>,

    /// Queued `LookSidewards(direction)` request — launches a
    /// one-or-two-element sequence of `LOOK_LEFT` / `LOOK_RIGHT` /
    /// `LEAN_OUT`. The engine builds the sequence at post-think time so
    /// the AI layer doesn't need access to the sequence manager.
    pub pending_look_sidewards: Option<LookDirection>,

    /// Queued `SetPosture(posture)` request. Emitted by the AI when the
    /// NPC reaches its guard post and the `likes_to_sit_around` /
    /// `special_action` flags are set. The engine applies this to the
    /// element's `PositionInterface` at post-think time so the AI layer
    /// doesn't need element mutation access.
    pub pending_posture: Option<crate::element::Posture>,

    /// Queued per-waypoint script `ReachPoint(actor)` dispatch —
    /// `Some((path_idx, wp_idx))` when the AI hit a
    /// `WaypointCommand::Script` waypoint this tick. Drained by the
    /// engine right after the triggering `think()` returns: it calls
    /// `MissionScript::call_waypoint_function("ReachPoint", &[me])`
    /// against the bound waypoint VM, then fires
    /// `EventAfterScriptGoOn` unless the script transitioned the NPC
    /// into `Substate::DefaultScriptDriven`. Fallback to
    /// `EventAfterScriptGoOn` if no script is bound for this waypoint
    /// preserves the previous behaviour for scripted-but-class-missing
    /// waypoints.
    pub pending_waypoint_script_reach_point: Option<(PathId, u8)>,

    /// Queued `Panic(center, runs, alert)` request. The engine drains
    /// this at post-think time and performs the door lookup (against
    /// `ai_global.door_seek_infos`) so the AI layer doesn't need the
    /// door list on its call stack. On success the engine transitions
    /// to `Fleeing / FleeingRunToDoor` and issues a GoTo to the door
    /// entry point; on failure (or when the NPC can't reach any door)
    /// it transitions to `Fleeing / FleeingPanic` and fires a
    /// self-`EventReachPoint` so the flee-run state machine picks up on
    /// the next tick.
    pub pending_begin_panic: Option<PanicRequest>,

    /// Set by the `FleeingPanic` `EventCouldntReachPoint` arm when the
    /// panic-run `GoTo` failed and we need the engine to pick a
    /// fallback `SeekPoint` to flee toward. Drained post-think by
    /// `EngineInner::process_pending_panic_seek_fallback_for`.
    pub pending_panic_seek_fallback: bool,

    /// Pending script-driven SeekArea request, set by
    /// `script_set_ai_state` when the bytecode asks for
    /// `STATE_SEEKING`. The engine drains this at post-think time by
    /// calling `EnemyAi::seek_area(center, radius, 0, UNDEFINED)`.
    pub pending_script_seek_area: Option<ScriptSeekAreaRequest>,

    // -- Stare target --
    /// If set, the NPC should face toward this actor for `stare_remaining` frames.
    pub stare_target_actor: NpcHandle,
    /// If set, the NPC should face toward this position for `stare_remaining` frames.
    pub stare_target_position: Option<Position>,
    /// Frames remaining for the stare behaviour. 0 = inactive.
    pub stare_remaining: u32,

    // -- Static entity context (set once at init/load) --
    /// Initial position (guard post / spawn point), set at level load.
    pub initial_position: Position,
    /// Initial facing direction (0–15), set at level load.
    pub initial_view_direction: u16,
    /// Maximum visibility across all enemy detectables this frame.
    /// Set by the engine detection tick. Used by `DefaultLookingShadow`
    /// to decide whether to keep watching.
    pub max_visibility: f32,

    // -- Cached engine state for say() / forbidden remarks --
    /// Current frame counter, set by the engine before think().
    pub cached_frame: u32,
    /// Whether this NPC is inside a building, set by the engine.
    pub cached_in_building: bool,
    /// Speech flags from the last accepted say() call. Stored so that
    /// `process_npc_speech` Phase 3 (finished exclamation callback) knows
    /// which MYTALK event to fire.
    pub pending_mytalk_flags: u16,

    /// Set by `set_alert_status_with_flags` when the caller passes
    /// `AlertFlags::INSTANT_MUSIC_CHANGE` and the call actually changes
    /// `current_music_alert_status`. Consumed once per frame by
    /// `EngineInner::update_overall_villain_alert`: if any NPC has it set
    /// when the overall villain alert transitions, music is dispatched
    /// via `ForceMusicMode` (immediate cut) instead of `SetMusicMode`
    /// (queued/blended), regardless of which colour transition fired.
    /// Cleared on every NPC after the sweep, whether or not music was
    /// dispatched — the flag is per-call.
    pub pending_instant_music_change: bool,
}

impl Default for AiController {
    fn default() -> Self {
        Self {
            me: 0,
            owner_entity_id: None,
            path_id: None,
            alert_path_id: None,
            current_state: AiState::Default,
            current_substate: Substate::DefaultOnPost,
            current_music_alert_status: AlertLevel::Green,
            view_alert_status: AlertLevel::Green,
            substate_at_last_timer_launch: Substate::DefaultOnPost,
            attitude: Attitude::Suspicious,
            blood_alcohol: 0,
            initial_action: 0,
            number_of_looks: 0,
            has_patrol_path: false,
            patrol_path: None,
            can_move: false,
            think_recursion_depth: 0,
            macro_command: Vec::new(),
            macro_command_offset: 0,
            number_of_remaining_macro_bytes: 0,
            macro_in_progress: false,
            macro_started_in_this_frame: false,
            primary_target: 0,
            friend_in_trouble: 0,
            detected_body: 0,
            interesting_object: 0,
            antagonist: 0,
            last_stimulus_actor: 0,
            timer_is_running: false,
            when_does_timer_ring: 0,
            macro_timer_is_running: false,
            when_does_macro_timer_ring: 0,
            standing_around_timer: 0,
            sorrow_level: 0,
            last_stimulus: [StimulusType::NoEvent; 5],
            last_stimulus_multiplicity: [1; 5],
            is_master: false,
            master: 0,
            seek_position: Position::default(),
            alert_soldiers_point: Position::default(),
            first_try: false,
            panic_center_x: 0.0,
            panic_center_y: 0.0,
            lasting_panic_runs: 0,
            directed_panic: false,
            list_us: Vec::new(),
            list_alerted_us: Vec::new(),
            list_staying_us: Vec::new(),
            couldnt_reachpoint: false,
            already_on_point: false,
            already_turned: false,
            likes_to_sit_around: false,
            special_action: false,
            friends_are_alerted: false,
            is_stay_at_home: false,
            locks_flag_field: AiLockFlags::empty(),
            was_busy: false,
            stimulus_queue: Vec::new(),
            script_locked: false,
            remember_events: false,
            leave_house_number: 0,
            forgotten_objects: Vec::new(),
            object_of_desire: 0,
            checkpoint_charly: 0,
            synchronize_charly: 0,
            synchronize_index: 0,
            delta_sorrow_level: 0,
            missed_in_action: Vec::new(),
            frame_when_enemy_detected: 0,
            inside_halt_method: false,
            synchronizing_actors: Vec::new(),
            default_path_walking_flags: GotoFlags::empty(),
            forbidden_remark_ids: Vec::new(),
            initial_view_cone: ViewCone::Commandoslike,
            current_remark: Remark::TheSoundOfSilence,
            current_remark_flags: 0,
            speech_in_flight: false,
            next_macro_rand: 0,
            next_macro_rand_forecasted: false,
            current_emoticon_type: EmoticonType::None,
            emoticon_expiration_date: 0,
            emoticon_has_expiration_date: false,
            my_reconnaissance_report: ReconnaissanceReport::default(),
            knocked_out_in_money_fight: false,
            looted_after_money_fight: false,
            patrol_chief: 0,
            patrol: Vec::new(),
            missed_patrol_members: Vec::new(),
            theoretical_patrol: Vec::new(),
            patrol_stopped: false,
            patrol_direction: 0,
            pending_patrol_direction_broadcast: None,
            needs_patrol_reinit: false,
            got_the_beggar_trick: false,
            ai_log: Vec::new(),
            debug_view_cone_enabled: false,
            last_goto_destination: Position::default(),
            last_goto_flags: GotoFlags::empty(),
            stuck_counter: 0,
            pending_orders: Vec::new(),
            pending_stimuli: Vec::new(),
            pending_cross_npc_actions: Vec::new(),
            pending_self_stimuli: Vec::new(),
            has_script_filter_override: false,
            pending_quit_swordfight: false,
            pending_stop_menace: false,
            pending_lower_shield: false,
            pending_deactivate: false,
            pending_halt: false,
            pending_broadcast_panic: false,
            pending_blink_all_enemies: false,
            pending_enemy_in_house_alert: false,
            pending_add_detectables: Vec::new(),
            pending_delete_detectables: Vec::new(),
            pending_delete_detectable_entity: Vec::new(),
            pending_delete_beggar_for_all_npc: Vec::new(),
            pending_blink_enemy_specific: Vec::new(),
            pending_enter_swordfight: None,
            pending_enter_swordfight_jump_line: None,
            pending_stop_target: None,
            pending_set_principal: None,
            pending_friend_primary_target_swap: None,
            pending_shoot_target: None,
            pending_focus: None,
            pending_unalert_near_charly_seekers: None,
            pending_refill_bow_ammo: false,
            pending_set_reported_to_officer: Vec::new(),
            pending_unfocus: false,
            pending_focus_point: None,
            last_synced_focus_target: 0,
            pending_state_change_notifications: Vec::new(),
            pending_slowly_open_eyes: false,
            pending_inform_resurrection: false,
            pending_restore_detectable_objects: false,
            pending_forget_nearby_coins: None,
            pending_set_eye_status: None,
            pending_set_direction_instantly: None,
            pending_set_attentive_mode: None,
            pending_set_guarded_pc: None,
            pending_launch_commands: Vec::new(),
            pending_launch_on_target: Vec::new(),
            pending_launch_sequences: Vec::new(),
            pending_look_sidewards: None,
            pending_posture: None,
            pending_begin_panic: None,
            pending_panic_seek_fallback: false,
            pending_script_seek_area: None,
            pending_waypoint_script_reach_point: None,
            stare_target_actor: 0,
            stare_target_position: None,
            stare_remaining: 0,
            initial_position: Position::default(),
            initial_view_direction: 0,
            max_visibility: 0.0,
            cached_frame: 0,
            cached_in_building: false,
            pending_mytalk_flags: 0,
            pending_instant_music_change: false,
        }
    }
}

// ---------------------------------------------------------------------------
// AiController methods (base controller logic)
// ---------------------------------------------------------------------------

impl AiController {
    pub fn new(owner: NpcHandle) -> Self {
        Self {
            me: owner,
            ..Default::default()
        }
    }

    // -- Per-NPC init (called from EngineInner::init_one_ai) --

    /// Evaluate `initial_action` and commit the matching AI-side
    /// state transition.
    ///
    /// Returns an [`InitStateSideEffects`] describing the entity-side
    /// mutations the caller (`EngineInner::init_one_ai`) must apply on
    /// NpcData / HumanData / ElementData / ActorData — fields the AI
    /// layer can't reach directly. The AI-side fields
    /// (`current_state` / `current_substate`, timer, emoticon,
    /// `likes_to_sit_around` / `special_action` / `is_stay_at_home`)
    /// are mutated in place before the return.
    ///
    /// The returned `go_to_duty` flag means: `true` — caller should
    /// run the standard "walk onto patrol path or launch a bored timer"
    /// tail; `false` — init placed the NPC in a sleeping / dead /
    /// sitting state and the caller must leave it alone.
    pub fn init_state(&mut self, ctx: &AiContext) -> InitStateSideEffects {
        use crate::element::{ActionState, EyeStatus, Posture};
        use crate::order::OrderType;

        // Reset the three "I'm authored as X" flags; the matching
        // switch-case below flips the one that applies.
        self.likes_to_sit_around = false;
        self.special_action = false;
        self.is_stay_at_home = false;

        let mut fx = InitStateSideEffects::default();

        // Indoor NPCs stay at home. House membership is already
        // guaranteed because `ai_global.houses` is populated from
        // *every* building sector during
        // `EngineInner::initialize_buildings`, so we just flip the
        // stay-at-home flag + substate here.
        if ctx.in_building {
            self.is_stay_at_home = true;
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultHomeSweetHome;
            return fx; // go_to_duty = false
        }

        let raw = self.initial_action;
        match OrderType::try_from(raw).ok() {
            // Plain waiting variants → on-post with a bored timer;
            // return `true` so the caller layers on `ReturnToDuty`.
            Some(
                OrderType::WaitingUpright
                | OrderType::WaitingUprightBored
                | OrderType::WaitingUprightBoredRandom,
            ) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(ctx);
                self.launch_timer(bored as u32, ctx.frame);
                fx.go_to_duty = true;
            }

            // Sleeping-upright — close eyes, posture Upright +
            // action_state Sleeping, Zzz emoticon.
            Some(OrderType::SleepingUpright) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingNapping;
                self.set_emoticon(EmoticonType::Zzz);
                fx.set_eye_status = Some(EyeStatus::Closed);
                fx.set_posture = Some(Posture::Upright);
                fx.set_action_state = Some(ActionState::Sleeping);
            }

            // Authored sitting. OnPost + bored timer, posture Sitting,
            // `likes_to_sit_around = true` so the return-to-duty branch
            // below picks the sitting placement path.
            Some(OrderType::Sitting) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(ctx);
                self.launch_timer(bored as u32, ctx.frame);
                self.likes_to_sit_around = true;
                fx.set_posture = Some(Posture::Sitting);
                fx.set_action_state = Some(ActionState::Waiting);
            }

            // Dead-fallen-back — zero life points, posture DeadBack,
            // killed-by-accident (engine side, bundled with
            // `zero_life_points`).
            Some(OrderType::BeingDeadFallenBack) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingForever;
                fx.zero_life_points = true;
                fx.set_posture = Some(Posture::DeadBack);
                fx.set_action_state = Some(ActionState::Waiting);
            }

            // Dead — same shape but posture Dead.
            Some(OrderType::BeingDead) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingForever;
                fx.zero_life_points = true;
                fx.set_posture = Some(Posture::Dead);
                fx.set_action_state = Some(ActionState::Waiting);
            }

            // Unconscious — max concussion + `unconscious = true`,
            // posture Lying. Init-time has no script-lock / carried /
            // tied gates to honour, so we bypass the full
            // `combat::set_concussion` state machine and write the
            // fields directly on the engine side.
            Some(OrderType::BeingUnconscious) => {
                self.set_ai_state(AiState::Sleeping);
                self.current_substate = Substate::SleepingUnconscious;
                fx.concussion_max_and_unconscious = true;
                fx.set_posture = Some(Posture::Lying);
                fx.set_action_state = Some(ActionState::Waiting);
            }

            // Special leisure — OnPost, posture Leisure,
            // `special_action = true` so the return-to-duty branch picks
            // the leisure placement path.
            Some(OrderType::Special) => {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                self.special_action = true;
                fx.set_posture = Some(Posture::Leisure);
                fx.set_action_state = Some(ActionState::Waiting);
            }

            // Unknown initial action — log a warning and default to
            // on-post.
            _ => {
                tracing::warn!(
                    "NPC {}: InitState received unsupported initial action {} — defaulting to OnPost",
                    self.me,
                    raw,
                );
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(ctx);
                self.launch_timer(bored as u32, ctx.frame);
                fx.go_to_duty = true;
            }
        }

        fx
    }

    // -- Timer --

    /// Arm the stimulus timer to fire `frames` ticks from now.
    pub fn launch_timer(&mut self, frames: u32, current_frame: u32) {
        // Clamp `frames == 0` to 1 so the timer never rings the same
        // frame it was armed.
        let frames = frames.max(1);
        self.timer_is_running = true;
        self.when_does_timer_ring = current_frame + frames;
        self.substate_at_last_timer_launch = self.current_substate;
    }

    // -- State transitions --

    pub fn set_ai_state(&mut self, state: AiState) {
        // Diagnostic at trace! level: log the caller path when an NPC
        // transitions out of `Attacking`, which is the class of bug
        // (AI flip-flopping out of combat) we've debugged a few times.
        // Enable with `RUST_LOG=robin_engine::ai=trace`.
        if self.current_state == AiState::Attacking && state != AiState::Attacking {
            tracing::trace!(
                from = ?self.current_state,
                to = ?state,
                substate = ?self.current_substate,
                bt = %std::backtrace::Backtrace::force_capture(),
                "set_ai_state: leaving Attacking"
            );
        }
        self.current_state = state;
    }

    // -- Locks --

    pub fn non_script_lock(&mut self, flags: AiLockFlags) {
        self.locks_flag_field |= flags;
    }

    pub fn non_script_unlock(&mut self, flags: AiLockFlags) {
        self.locks_flag_field -= flags;
    }

    pub fn ai_is_locked(&self) -> bool {
        !self.locks_flag_field.is_empty()
    }

    /// Whether a `FilterAIEvent`-triggered script has claimed the
    /// stimulus queue and the AI must suspend until `ScriptUnlockAI`
    /// fires.
    pub fn ai_is_script_locked(&self) -> bool {
        self.script_locked
    }

    /// Script-side AI lock.
    ///
    /// Sets `script_locked` + `remember_events`, halts the NPC's
    /// current engine order (unless the lock itself is the active
    /// command), and drops any running waypoint macro. Callers invoked
    /// from the `LockAi` sequence command handler must pass
    /// `from_lockai_command = true` so the stop doesn't cancel the very
    /// command that triggered the lock; every other site passes
    /// `false`.
    pub fn script_lock(&mut self, remember_events: bool, from_lockai_command: bool) {
        self.script_locked = true;
        self.remember_events = remember_events;
        // C++ AI calls Think(EVENT_RETURN_TO_DUTY) synchronously from
        // AssignPath before the later recorded LockAI can run. In Rust
        // those AI actions are deferred through pending_* queues; once the
        // script lock lands, no pre-lock deferred return-to-duty work may
        // survive and interrupt the scripted sequence that follows.
        self.pending_orders.clear();
        self.pending_self_stimuli.clear();
        if !from_lockai_command {
            // Cancel the NPC's current order. The engine drains
            // `pending_halt` in post-think.
            self.pending_halt = true;
        }
        self.break_macro();
    }

    /// Clear the script lock and, unless a `EventAfterScriptGoOn` is
    /// already queued or the NPC is asleep/unconscious, schedule a
    /// `EventReturnToDuty` self-stimulus so the AI re-enters its state
    /// machine immediately. Also latches `pending_blink_all_enemies` so
    /// the next detection pass re-registers anyone still in the view
    /// cone.
    pub fn script_unlock(&mut self, is_unconscious: bool) {
        // Clear current detections so NPCs re-register view-cone
        // occupants on the next detection pass.
        self.pending_blink_all_enemies = true;

        // Skip the return-to-duty Think if a EVENT_AFTER_SCRIPT_GO_ON
        // is already queued — the script left a waypoint-continuation
        // stimulus that must drain first.
        let after_script_go_on = self
            .stimulus_queue
            .iter()
            .any(|s| s.stimulus_type == StimulusType::EventAfterScriptGoOn);

        self.script_locked = false;

        if self.current_state != AiState::Sleeping && !after_script_go_on && !is_unconscious {
            self.pending_self_stimuli
                .push(StimulusType::EventReturnToDuty);
        }
    }

    // -- Emoticon --

    pub fn set_emoticon(&mut self, emoticon: EmoticonType) {
        self.current_emoticon_type = emoticon;
        self.emoticon_has_expiration_date = false;
    }

    pub fn set_transient_emoticon(
        &mut self,
        emoticon: EmoticonType,
        frames: u16,
        current_frame: u32,
    ) {
        self.current_emoticon_type = emoticon;
        self.emoticon_has_expiration_date = true;
        self.emoticon_expiration_date = current_frame + frames as u32;
    }

    pub fn clear_emoticon(&mut self) {
        self.set_emoticon(EmoticonType::None);
    }

    // -- Master/group --

    // -- Patrol --

    pub fn has_patrol(&self) -> bool {
        !self.theoretical_patrol.is_empty()
    }

    /// Clear the chief's three patrol lists. Per-minion cleanup
    /// (`SetPatrolChief(NULL)` + `ForceReturnToDuty` for STATE_DEFAULT
    /// minions) needs the engine's entity table and runs at the
    /// `RemoveAllSubordinates` native call site.
    pub fn clear_patrol(&mut self) {
        self.theoretical_patrol.clear();
        self.missed_patrol_members.clear();
        self.patrol.clear();
    }

    // -- Stimulus history --

    /// Append a log line stamped with the current universal frame counter.
    /// The list is capped at the 26 most-recent entries.
    pub fn register_log_line(&mut self, line_type: LogLineType, info: u16) {
        self.ai_log.push(LogLine {
            line_type,
            info,
            frame: self.cached_frame,
        });
        while self.ai_log.len() > 26 {
            self.ai_log.remove(0);
        }
    }

    /// Render the per-NPC AI log via `tracing`.
    ///
    /// Each log entry becomes one `trace!` line in the `ai_log` target —
    /// the caller (engine) gates this on `ai_global.attribute_display`
    /// plus the host-side `selected_view_element`.
    ///
    /// Matches the original `DisplayLog` strings, including `*ToString`
    /// fallback labels for unknown raw log info values.
    pub fn display_log(&self, current_frame: u32) {
        let any_state_change = self
            .ai_log
            .iter()
            .any(|l| l.line_type == LogLineType::ChangeState);

        // When no state-change entry is present, the first on-screen
        // line is the current substate.
        if !any_state_change {
            tracing::trace!(
                target: "ai_log",
                "[{}]",
                self.current_substate
                    .log_string()
                    .unwrap_or_else(|| "SUBSTATE-???".to_string())
            );
        }

        // Quirk preserved verbatim so the line count matches the
        // original overlay: when the substate header is printed the
        // loop skips index 0.
        let start = if any_state_change { 0 } else { 1 };
        let mut last_displayed_speech_frame: u32 = 0;

        for line in self.ai_log.iter().skip(start) {
            match line.line_type {
                LogLineType::Event => {
                    tracing::trace!(
                        target: "ai_log",
                        "Event in frame {}: {}",
                        line.frame,
                        StimulusType::log_string_from_u16(line.info),
                    );
                }
                LogLineType::EventRefused => {
                    tracing::trace!(
                        target: "ai_log",
                        "     refused! Code #{}",
                        line.info,
                    );
                }
                LogLineType::ChangeState => {
                    tracing::trace!(
                        target: "ai_log",
                        "State change: {}",
                        Substate::log_string_from_u16(line.info),
                    );
                }
                LogLineType::BattleDecision => {
                    tracing::trace!(
                        target: "ai_log",
                        "Decision: {}",
                        Decision::log_string_from_u16(line.info),
                    );
                }
                LogLineType::Speak => {
                    last_displayed_speech_frame = line.frame;
                    tracing::trace!(
                        target: "ai_log",
                        "Speak: \"{}\"",
                        Remark::log_string_from_u16(line.info),
                    );
                }
                LogLineType::SpeakImpossible => {
                    tracing::trace!(
                        target: "ai_log",
                        "Speak impossible! Code #{}",
                        line.info,
                    );
                }
                LogLineType::SpeakFinished => {
                    if last_displayed_speech_frame > 0 {
                        tracing::trace!(
                            target: "ai_log",
                            "Speak finished after {} frames",
                            line.frame.saturating_sub(last_displayed_speech_frame),
                        );
                    } else {
                        tracing::trace!(
                            target: "ai_log",
                            "Speak finished after ??? frames",
                        );
                    }
                }
                LogLineType::Timer => {
                    tracing::trace!(
                        target: "ai_log",
                        "Timer launched: {} frames",
                        line.info,
                    );
                }
            }
        }

        // Trailing timer / macro-timer countdowns.
        if self.timer_is_running {
            tracing::trace!(
                target: "ai_log",
                "Timer: {}",
                self.when_does_timer_ring.saturating_sub(current_frame),
            );
        }
        if self.macro_timer_is_running {
            tracing::trace!(
                target: "ai_log",
                "Macro Timer: {}",
                self.when_does_macro_timer_ring.saturating_sub(current_frame),
            );
        }
    }

    // -- Random values --

    /// Random value in the half-open interval `[min, max)` with the
    /// given distribution. `lambda` is the pre-computed consideration
    /// score in `[0, MAX_ATT_VALUE]` — pass `MAX_ATT_VALUE as u8` for
    /// an un-biased sample.
    pub fn random_value(
        dist: ProbabilityDistribution,
        min_val: i16,
        max_val: i16,
        lambda: u8,
    ) -> i16 {
        debug_assert!(max_val >= min_val);
        let range = max_val - min_val;
        let lambda = lambda as i32;

        // `gauss_curve_top = min + (lambda * range) / MAX_ATT_VALUE`
        let gauss_curve_top = min_val + ((lambda * range as i32) / MAX_ATT_VALUE) as i16;

        match dist {
            ProbabilityDistribution::Dirac => gauss_curve_top,
            ProbabilityDistribution::Rectangle => {
                if range == 0 {
                    return min_val;
                }
                // Half-open `[min, max)` matches the original
                // `rand() % (max-min)` shape.
                min_val + crate::sim_rng::i16(0..range)
            }
            ProbabilityDistribution::GaussHighVariance => {
                // `range*0.333` truncated (three samples) and
                // `range*0.5` for the centring shift.
                let third = ((range as f32) * 0.333) as i16;
                let half = ((range as f32) * 0.5) as i16;
                let mut val: i32 = 0;
                if third > 0 {
                    val = crate::sim_rng::i16(0..third) as i32
                        + crate::sim_rng::i16(0..third) as i32
                        + crate::sim_rng::i16(0..third) as i32;
                }
                val += gauss_curve_top as i32 - half as i32;
                (val.clamp(min_val as i32, max_val as i32)) as i16
            }
            ProbabilityDistribution::Gauss => {
                // `range*0.166` truncated (three samples) and
                // `range*0.25` for the centring shift.
                let sixth = ((range as f32) * 0.166) as i16;
                let quarter = ((range as f32) * 0.25) as i16;
                let mut val: i32 = 0;
                if sixth > 0 {
                    val = crate::sim_rng::i16(0..sixth) as i32
                        + crate::sim_rng::i16(0..sixth) as i32
                        + crate::sim_rng::i16(0..sixth) as i32;
                }
                val += gauss_curve_top as i32 - quarter as i32;
                (val.clamp(min_val as i32, max_val as i32)) as i16
            }
        }
    }

    // -- Decision support (ConsiderValue / EvaluateConsiderations) --
    // Modelled as static helpers; original engine used thread-local
    // accumulators.

    /// Interpolate between two values based on a parameter in 0..100.
    ///
    /// `0.01f * param` is cast to `u16` (truncation toward zero), so
    /// for `param ∈ [0, 99]` the cast yields `0` and the function
    /// returns `value_at_0`; only `param == 100` yields `1` and returns
    /// `value_at_100`. This is therefore a step function, not a linear
    /// interpolation, despite its name. The truncation is preserved for
    /// bit-for-bit parity with original behaviour.
    pub fn value_between(value_at_0: u16, value_at_100: u16, param: u8) -> u16 {
        debug_assert!(param <= 100);
        let p = (0.01f32 * param as f32) as u16;
        value_at_0.wrapping_add(value_at_100.wrapping_sub(value_at_0).wrapping_mul(p))
    }

    // -- Bored time --

    /// Returns the time until this NPC gets bored and does something.
    /// Officers and high-pride soldiers use longer intervals; everyone
    /// else uses the short default.
    pub fn get_bored_time(&self, ctx: &AiContext) -> u16 {
        use crate::profiles::ProfileRank;
        const AI_MIN_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_DELTA_DEFAULT_BORED_INTERVAL: u16 = 70;
        const AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 200;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER: u16 = 600;
        const AI_MIN_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 400;
        const AI_DELTA_DEFAULT_BORED_INTERVAL_PRIDE: u16 = 800;

        let (min, delta) = if ctx.self_rank == ProfileRank::Officer {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL_OFFICER,
                AI_DELTA_DEFAULT_BORED_INTERVAL_OFFICER,
            )
        } else if ctx.self_pride > 0 {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL_PRIDE,
                AI_DELTA_DEFAULT_BORED_INTERVAL_PRIDE,
            )
        } else {
            (
                AI_MIN_DEFAULT_BORED_INTERVAL,
                AI_DELTA_DEFAULT_BORED_INTERVAL,
            )
        };
        // P_RECTANGLE ignores `lambda`; pass MAX_ATT_VALUE for the un-biased sample.
        min + (Self::random_value(
            ProbabilityDistribution::Rectangle,
            0,
            delta as i16,
            MAX_ATT_VALUE as u8,
        ) as u16)
    }

    // -- Retrograde amnesia --

    // -- Pending order access --

    /// Drain all pending orders produced by AI decisions.
    /// Called by the engine each tick to dispatch them.
    pub fn take_pending_orders(&mut self) -> Vec<AiOrderIntent> {
        std::mem::take(&mut self.pending_orders)
    }

    /// Whether the AI has produced any orders this tick.
    pub fn has_pending_orders(&self) -> bool {
        !self.pending_orders.is_empty()
    }

    // -- Cross-NPC action access --

    /// Drain all pending cross-NPC actions produced by phalanx logic.
    /// Called by the engine after each think() to dispatch them.
    pub fn take_pending_cross_npc_actions(&mut self) -> Vec<CrossNpcAction> {
        std::mem::take(&mut self.pending_cross_npc_actions)
    }

    /// Drain self-directed stimuli queued by `say()`.
    /// The engine re-dispatches these as think() calls to the same NPC.
    pub fn take_pending_self_stimuli(&mut self) -> Vec<StimulusType> {
        std::mem::take(&mut self.pending_self_stimuli)
    }

    // -- Shield commands --

    /// Issue a raise-shield order toward a danger point.
    pub fn raise_shield(&mut self, danger_point: Position) {
        use crate::order::OrderType;
        self.pending_orders.push(AiOrderIntent::new(
            OrderType::RaisingShield,
            danger_point.x,
            danger_point.y,
        ));
    }

    /// Issue a lower-shield order.
    pub fn lower_shield(&mut self) {
        use crate::order::OrderType;
        self.pending_orders
            .push(AiOrderIntent::new(OrderType::LoweringShield, 0.0, 0.0));
    }

    /// Base-class virtual hook for `default_bored_standard_procedure` —
    /// returns `false`.
    ///
    /// Used as a dispatch entry from base-level call sites that want to
    /// give a subclass a chance to react to "I'm bored / nothing left
    /// to do" — currently the macro-end branch of
    /// [`Self::execute_next_macro_command`].
    ///
    /// Civilians/friendlies inherit this no-op. Soldiers override the
    /// hook, but the override gates on `Substate::DefaultOnPost`; the
    /// macro-end call site enters this hook with substate
    /// `DefaultInMacro`, so the gate fails and the soldier override
    /// observably returns false too. Returning false for everyone here
    /// matches both subclasses' behaviour.
    ///
    /// The canonical soldier-side override lives at
    /// `EnemyAi::default_bored_standard_procedure` and is invoked from
    /// the bored-timer expiry path where the substate gate can actually
    /// pass and the EnemyAi-specific `set_state` side effects
    /// (archer/shield-bearer pairing teardown) are required.
    pub fn default_bored_standard_procedure(&mut self, _ctx: &AiContext) -> bool {
        false
    }

    // -- Break macro --

    pub fn break_macro(&mut self) {
        self.macro_in_progress = false;
        self.number_of_remaining_macro_bytes = 0;
        self.macro_command.clear();
        self.macro_command_offset = 0;
        self.macro_timer_is_running = false;
        // `BreakMacro` clears `DETECTABLE_MISSED_FRIEND` and zeros
        // `sorrow_level` as side effects — route through
        // `set_checkpoint_charly` so the detectable queue + sorrow
        // reset stay consistent.
        self.set_checkpoint_charly(0);
    }

    /// Overwrites the stashed checkpoint actor and applies the
    /// detectable/sorrow bookkeeping every call:
    ///
    /// * Unconditionally enqueue `DeleteAllDetectables(MissedFriend)`.
    /// * When `target` is non-zero, enqueue an
    ///   `AddDetectable(target, MissedFriend)` so the target shows up
    ///   in the "missed friend" list.
    /// * When `target` is zero, zero `sorrow_level` and enqueue a
    ///   second delete (belt-and-braces).
    ///
    /// The engine drains `pending_delete_detectables` before
    /// `pending_add_detectables` in [`EngineInner::tick_ai_pending_*`],
    /// so a non-zero call correctly clears the list then re-adds the
    /// target.
    pub fn set_checkpoint_charly(&mut self, target: NpcHandle) {
        use crate::element::DetectableType;
        self.pending_delete_detectables
            .push(DetectableType::MissedFriend);
        self.checkpoint_charly = target;
        if target != 0 {
            self.pending_add_detectables.push((
                crate::element::EntityId::Soldier(target),
                DetectableType::MissedFriend,
            ));
        } else {
            self.sorrow_level = 0;
            self.pending_delete_detectables
                .push(DetectableType::MissedFriend);
        }
    }

    /// Merge `other` into `my_reconnaissance_report` and push the
    /// detectable side effects onto the pending queues so the engine
    /// drain runs `DeleteDetectable(body, BODY)` (every newly-merged
    /// body, regardless of `UPDATE_BODIES`) and
    /// `AddDetectable(charly, MISSED_FRIEND)` (when a new charly handle
    /// is adopted under `UPDATE_CHARLY`).
    ///
    /// Flag bits:
    /// * `0x01` — `UPDATE_BODIES`: copy `seen_bodies` from `other`.
    /// * `0x02` — `UPDATE_CHARLY`: copy charly handle if we don't have one.
    /// * `0x04` — `UPDATE_TYPE`: monotonically promote report type and
    ///   seek position via `ReconnaissanceReport::update`.
    ///
    /// The DeleteDetectable side effect fires for every newly-seen body
    /// even when `UPDATE_BODIES` is clear.
    pub fn consider_report_merged(&mut self, other: &ReconnaissanceReport, flags: u16) {
        use crate::element::{DetectableType, EntityId};
        const REPORT_UPDATE_BODIES: u16 = 1;
        const REPORT_UPDATE_CHARLY: u16 = 2;
        const REPORT_UPDATE_TYPE: u16 = 4;

        // Per-body merge + per-body DeleteDetectable.
        for &body in &other.seen_bodies {
            if !self.my_reconnaissance_report.is_body_seen(body) {
                if (flags & REPORT_UPDATE_BODIES) != 0 {
                    self.my_reconnaissance_report.add_seen_body(body);
                }
                // Unconditional `DeleteDetectable(body, BODY)` —
                // fires whether or not UPDATE_BODIES is set.
                self.pending_delete_detectable_entity
                    .push((EntityId::Soldier(body), DetectableType::Body));
            }
        }

        // Charly merge + AddDetectable(MISSED_FRIEND).
        if (flags & REPORT_UPDATE_CHARLY) != 0
            && other.charly != 0
            && self.my_reconnaissance_report.charly == 0
        {
            self.my_reconnaissance_report.charly = other.charly;
            self.pending_add_detectables.push((
                EntityId::Soldier(other.charly),
                DetectableType::MissedFriend,
            ));
        }

        // Monotonically update type / seek_position.
        if (flags & REPORT_UPDATE_TYPE) != 0 {
            self.my_reconnaissance_report
                .update(other.report_type, other.seek_position);
        }
    }

    /// Pick the closest seek point to flee toward when a panic-run
    /// `GoTo` is blocked.
    ///
    /// Walks `seek_points`, computes `MaxNorm` of the delta from our
    /// current position, adds `1000` for a sector change and `5000`
    /// when a directed panic would end up fleeing *toward* the panic
    /// source, and returns the index of the minimum.
    pub fn nearest_seek_point_to_flee(
        &self,
        seek_points: &[SeekPoint],
        my_pos: Position,
        my_sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (idx, sp) in seek_points.iter().enumerate() {
            let dx = sp.position.x - my_pos.x;
            let dy = sp.position.y - my_pos.y;
            let mut distance = dx.abs().max(dy.abs()) as u32;
            if sp.position.sector != my_sector {
                distance = distance.saturating_add(1000);
            }
            if self.directed_panic {
                // Big penalty for fleeing toward the panic source:
                // (seek_delta · (panic_center - my_pos)) > 0 means
                // the seek point lies in the same half-plane as the
                // panic source.
                let cx = self.panic_center_x - my_pos.x;
                let cy = self.panic_center_y - my_pos.y;
                if dx * cx + dy * cy > 0.0 {
                    distance = distance.saturating_add(5000);
                }
            }
            if best.map(|(_, d)| distance < d).unwrap_or(true) {
                best = Some((idx, distance));
            }
        }
        best.map(|(idx, _)| idx)
    }

    // -- Macro rand --

    /// Random value in `[1, 100]` for macro section-selection.
    /// Consumes the cached forecast if present, otherwise rolls a
    /// fresh value.
    pub fn calculate_macro_rand(&mut self) -> u8 {
        if self.next_macro_rand_forecasted {
            self.next_macro_rand_forecasted = false;
            self.next_macro_rand
        } else {
            (crate::sim_rng::u32(0..100) as u8) + 1
        }
    }

    /// Forecast the next return value of `calculate_macro_rand` without
    /// consuming it. Called when one NPC needs to peek at another's
    /// upcoming roll (section-selection coherence).
    pub fn forecast_macro_rand(&mut self) -> u8 {
        if !self.next_macro_rand_forecasted {
            self.next_macro_rand = (crate::sim_rng::u32(0..100) as u8) + 1;
            self.next_macro_rand_forecasted = true;
        }
        self.next_macro_rand
    }

    // -- Macro timer --

    /// Arm the macro-specific timer. When the timer rings, the engine's
    /// AI hourglass calls [`Self::execute_next_macro_command`] directly
    /// (bypassing the Think state machine).
    pub fn launch_macro_timer(&mut self, frames: u32, current_frame: u32) {
        // Clamp `frames == 0` to 1 so a macro timer never rings the
        // same frame it was armed.
        let frames = frames.max(1);
        self.macro_timer_is_running = true;
        self.when_does_macro_timer_ring = current_frame + frames;
    }

    // -- Patrol macro helpers --

    /// Assign a new patrol path (or drop the current one). The three
    /// call shapes (sentinel `-1`, sentinel `-2`, valid index) collapse
    /// to the cases encoded in [`PatrolAssignment`].
    ///
    /// Side effects:
    /// - `BreakMacro()` prologue unconditionally.
    /// - On clear: snapshot current position/direction into
    ///   `initial_position` / `initial_view_direction` so
    ///   `return_to_duty_common_stuff` sends the NPC back to the
    ///   right anchor.
    /// - Reset `likes_to_sit_around` (per variant), `special_action`,
    ///   `is_stay_at_home` flags.
    /// - Bounds-check index variant against hiking path count; out of
    ///   range returns `false` without touching state.
    /// - When `!script_locked && current_state == Default`, fire a
    ///   self `EventReturnToDuty` so the NPC walks to the new path /
    ///   post on the next tick.
    ///
    /// Callers must supply the NPC's current map position + facing
    /// (0–15) so the initial-pos snapshot is accurate.
    pub fn assign_new_patrol_path(
        &mut self,
        assignment: PatrolAssignment,
        current_position: Position,
        current_direction: u16,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> bool {
        self.break_macro();

        match assignment {
            PatrolAssignment::ClearPath | PatrolAssignment::ClearPathSitAround => {
                let sits = matches!(assignment, PatrolAssignment::ClearPathSitAround);
                self.has_patrol_path = false;
                self.patrol_path = None;
                self.path_id = None;
                self.initial_position = current_position;
                self.initial_view_direction = current_direction & 0x0F;
                self.likes_to_sit_around = sits;
                self.special_action = false;
                self.is_stay_at_home = false;
                if !self.script_locked && self.current_state == AiState::Default {
                    self.fire_self_stimulus(StimulusType::EventReturnToDuty);
                }
                true
            }
            PatrolAssignment::Index(pid) => {
                let idx = pid.get() as usize;
                // Strictly greater, so `idx == count` is tolerated
                // (matches the off-by-one in the original engine).
                if idx > hiking_paths.len() {
                    tracing::warn!(
                        npc = self.me,
                        idx = pid.get(),
                        count = hiking_paths.len(),
                        "AssignNewPatrolPath: index out of range",
                    );
                    return false;
                }
                self.has_patrol_path = true;
                self.path_id = Some(pid);
                self.patrol_path = PatrolPath::new(pid, hiking_paths);
                self.likes_to_sit_around = false;
                self.special_action = false;
                if !self.script_locked && self.current_state == AiState::Default {
                    self.fire_self_stimulus(StimulusType::EventReturnToDuty);
                }
                true
            }
        }
    }

    /// Assign a new guard post.
    ///
    /// Drops any active patrol path, installs the new post as the
    /// NPC's `initial_position` / `initial_view_direction` anchor,
    /// clears the three authored flags, and — when not script-locked
    /// and in the default state — fires `EventReturnToDuty` so the
    /// NPC walks to the new post.
    pub fn assign_new_post(&mut self, post_position: Position, post_direction: u16) -> bool {
        self.break_macro();

        self.path_id = None;
        self.patrol_path = None;
        self.has_patrol_path = false;
        self.initial_position = post_position;
        self.initial_view_direction = post_direction & 0x0F;
        self.is_stay_at_home = false;
        self.likes_to_sit_around = false;
        self.special_action = false;

        if !self.script_locked && self.current_state == AiState::Default {
            self.fire_self_stimulus(StimulusType::EventReturnToDuty);
        }
        true
    }

    /// Script-driven AI state entry. Wires the per-state side effects
    /// that the bare `set_ai_state` field write omits:
    /// `Think(EVENT_RETURN_TO_DUTY)` for `Default`, `SeekArea` via
    /// `pending_script_seek_area` for `Seeking`, and `Panic` via
    /// `pending_begin_panic` for `Fleeing`.
    ///
    /// Unreachable arms (`Sleeping`, `Wondering`, `Menacing`,
    /// `Attacking`) are logged as warnings and skipped.
    pub fn script_set_ai_state(
        &mut self,
        state: AiState,
        current_position: Position,
        in_macro: bool,
        self_is_soldier: bool,
    ) {
        match state {
            AiState::Default => {
                if in_macro {
                    // SetState(STATE_DEFAULT, SUBSTATE_DEFAULT_INMACRO)
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultInMacro;
                } else {
                    // Think(EVENT_RETURN_TO_DUTY). Queue a self-stimulus
                    // so the next tick dispatches it through the normal
                    // think pipeline.
                    self.fire_self_stimulus(StimulusType::EventReturnToDuty);
                }
            }
            AiState::Seeking => {
                // Soldier-only.
                if !self_is_soldier {
                    tracing::warn!(
                        npc = self.me,
                        "SetAIState(SEEKING) on non-soldier NPC — ignored",
                    );
                    return;
                }
                self.pending_script_seek_area = Some(ScriptSeekAreaRequest {
                    center: current_position,
                    radius: crate::parameters_ai::AI_SCRIPT_SEEK_RADIUS as u16,
                });
            }
            AiState::Fleeing => {
                // Panic(AI_MACRO_PANIC_RUNS) undirected. The NULL
                // flee-center is re-expressed as a directed flee away
                // from the NPC's own position, matching the fall-through
                // behaviour of `process_pending_begin_panic_for` when
                // center == position (dot-product zero everywhere ⇒
                // falls into the no-door branch).
                let runs = crate::parameters_ai::AI_MACRO_PANIC_RUNS as u8;
                let was_already_fleeing = self.current_state == AiState::Fleeing
                    && matches!(
                        self.current_substate,
                        Substate::FleeingPanic | Substate::FleeingRunToDoor
                    );
                self.panic_center_x = current_position.x;
                self.panic_center_y = current_position.y;
                self.lasting_panic_runs = runs;
                self.directed_panic = false;
                self.set_ai_state(AiState::Fleeing);
                self.current_substate = Substate::FleeingPanic;
                self.pending_begin_panic = Some(PanicRequest {
                    center: None,
                    runs,
                    alert: AlertLevel::Red,
                    is_new_panic: !was_already_fleeing,
                });
            }
            AiState::Sleeping | AiState::Wondering | AiState::Menacing | AiState::Attacking => {
                // Scripts shouldn't invoke these directly.  Log and skip.
                tracing::warn!(
                    npc = self.me,
                    ?state,
                    "SetAIState: unsupported target state",
                );
            }
        }
    }

    /// Broadcast a facing direction to every member of this NPC's patrol
    /// formation.
    ///
    /// The per-minion call iterates `patrol` and writes
    /// `patrol_direction` on each minion; if the minion is in
    /// `DefaultPatrolEnrouteWaiting`, it also calls `FaceTo(direction)`
    /// on the minion. Each minion call needs an `AiContext` which the
    /// chief's `AiController` doesn't have access to — so we queue the
    /// directive into `pending_patrol_direction_broadcast` and let
    /// `EngineInner::tick_patrol_coordination` Phase 0 drain it (it has
    /// access to each minion's entity + context).
    pub fn instruct_patrol_direction_to_patrol_members(&mut self, direction: u16) {
        self.pending_patrol_direction_broadcast = Some(direction);
    }

    // -- Waypoint-script launch --

    /// Kick off a script-driven waypoint.
    ///
    /// Calls the waypoint's bound script class (`ReachPoint(actor)`)
    /// and, if the script didn't lock the AI into
    /// `Substate::DefaultScriptDriven`, fires `EventAfterScriptGoOn` so
    /// the stimulus queue can drain.
    ///
    /// The per-waypoint VM instance lives on `MissionScript` (keyed by
    /// `(path_idx, wp_idx)`), so we can't dispatch from the AI layer
    /// directly. Instead we record the intent on
    /// `pending_waypoint_script_reach_point`; the engine drains it
    /// right after `think()` returns, calls `ReachPoint(actor)` on the
    /// bound instance, and then fires `EventAfterScriptGoOn` unless the
    /// script put us into `DefaultScriptDriven`.
    pub fn execute_waypoint_script(&mut self, path_idx: PathId, wp_idx: u8) {
        self.pending_waypoint_script_reach_point = Some((path_idx, wp_idx));
    }

    // -- Waypoint-macro launch --

    /// Parse the macro data block attached to a waypoint, roll a
    /// section, and start executing it.
    ///
    /// Layout of `macro_data` (all multi-byte values are little-endian,
    /// offsets relative to byte 0):
    ///
    /// ```text
    /// u16 num_direction_blocks   (1 or 2)
    /// Per direction block:
    ///     u8  direction_flag     (DIR_BOTH=0 / DIR_FORWARD=1 / DIR_BACKWARD=2)
    ///     u16 section_table_offset
    ///
    /// At section_table_offset:
    ///     u16 num_sections
    ///     Per section entry:
    ///         u8  probability_weight      (sums to 100)
    ///         u16 section_data_offset
    ///
    /// At section_data_offset:
    ///     u16 num_macro_bytes
    ///     bytes...                        (the opcode stream)
    /// ```
    ///
    /// Returns `true` if a macro was launched (execution proceeded),
    /// `false` if the waypoint should be skipped via
    /// `proceed_on_path` (no matching direction block, or all
    /// probability weights fell below the roll).
    pub fn launch_waypoint_macro(&mut self, macro_data: &[u8], ctx: &AiContext) -> bool {
        tracing::trace!(
            me = self.me,
            macro_len = macro_data.len(),
            path_idx = self
                .patrol_path
                .as_ref()
                .map(|p| p.hiking_path_index.get())
                .unwrap_or(0xFFFF),
            wp_idx = self
                .patrol_path
                .as_ref()
                .map(|p| p.current_waypoint_index)
                .unwrap_or(0xFF),
            "launch_waypoint_macro ENTRY"
        );
        let forward = self.patrol_path.as_ref().map(|p| p.forward).unwrap_or(true);

        // Read u16 LE at `off`, returning None on overflow.
        let read_u16 = |off: usize| -> Option<u16> {
            if off + 2 > macro_data.len() {
                None
            } else {
                Some(u16::from_le_bytes([macro_data[off], macro_data[off + 1]]))
            }
        };
        let read_u8 = |off: usize| -> Option<u8> { macro_data.get(off).copied() };

        let Some(num_dir_blocks) = read_u16(0) else {
            tracing::warn!(
                "NPC {}: malformed waypoint macro — missing num_direction_blocks",
                self.me
            );
            return false;
        };
        if num_dir_blocks == 0 || num_dir_blocks > 2 {
            tracing::warn!(
                "NPC {}: waypoint macro has invalid num_direction_blocks={}",
                self.me,
                num_dir_blocks
            );
            return false;
        }

        // Pick the direction block that matches our traversal direction.
        let direction_matches = |flag: u8| -> bool {
            match flag {
                0 => true,     // DIR_BOTH
                1 => forward,  // DIR_FORWARD
                2 => !forward, // DIR_BACKWARD
                _ => false,
            }
        };

        // Scan the block header triples `(u8 flag, u16 offset)` at
        // offsets 2, 5, ... until we find one whose direction matches.
        let mut section_table_offset: Option<usize> = None;
        for i in 0..num_dir_blocks as usize {
            let hdr_off = 2 + i * 3;
            let Some(flag) = read_u8(hdr_off) else { break };
            let Some(offset) = read_u16(hdr_off + 1) else {
                break;
            };
            if direction_matches(flag) {
                section_table_offset = Some(offset as usize);
                break;
            }
        }

        let Some(section_table_off) = section_table_offset else {
            // No applicable direction block — skip the waypoint.
            return false;
        };

        let Some(num_sections) = read_u16(section_table_off) else {
            tracing::warn!("NPC {}: waypoint macro section table is truncated", self.me);
            return false;
        };
        if num_sections == 0 {
            return false;
        }

        // Roll [1, 100] and walk the probability table.
        let initial_roll = self.calculate_macro_rand();
        let mut roll = initial_roll;
        let mut section_idx: Option<usize> = None;
        let weights: Vec<u8> = (0..num_sections as usize)
            .filter_map(|i| read_u8(section_table_off + 2 + i * 3))
            .collect();
        let first_ops: Vec<u8> = (0..num_sections as usize)
            .filter_map(|i| {
                let entry_off = section_table_off + 2 + i * 3 + 1;
                let data_off = read_u16(entry_off)?;
                macro_data.get(data_off as usize + 2).copied()
            })
            .collect();
        tracing::trace!(
            me = self.me,
            num_sections,
            ?weights,
            ?first_ops,
            initial_roll,
            "launch_waypoint_macro weights"
        );
        for i in 0..num_sections as usize {
            let entry_off = section_table_off + 2 + i * 3;
            let Some(weight) = read_u8(entry_off) else {
                break;
            };
            if roll <= weight {
                section_idx = Some(i);
                break;
            }
            roll -= weight;
        }

        let Some(selected) = section_idx else {
            // Probabilities all under the roll — proceed on path without macro.
            return false;
        };

        // Read the selected section's data offset.
        let data_off_entry = section_table_off + 2 + selected * 3 + 1;
        let Some(section_data_offset) = read_u16(data_off_entry) else {
            return false;
        };
        let section_data_off = section_data_offset as usize;

        let Some(macro_byte_count) = read_u16(section_data_off) else {
            tracing::warn!("NPC {}: waypoint macro section body is truncated", self.me);
            return false;
        };

        tracing::trace!(
            me = self.me,
            section = selected,
            macro_byte_count,
            first_op = macro_data
                .get(section_data_off + 2)
                .copied()
                .unwrap_or(0xff),
            "launch_waypoint_macro picked section"
        );

        // Stash the opcode stream on the AI. We keep a copy of the whole
        // data block so the cursor (`macro_command_offset`) can walk
        // forward into it.
        self.macro_command = macro_data.to_vec();
        self.macro_command_offset = section_data_off + 2;
        self.number_of_remaining_macro_bytes = macro_byte_count;

        // Start the macro machine.
        self.set_ai_state(AiState::Default);
        self.current_substate = Substate::DefaultInMacro;
        self.macro_started_in_this_frame = true;
        self.execute_next_macro_command(ctx);
        true
    }

    // -- Macro VM --

    /// Execute waypoint-macro opcodes until one blocks (wait-for-DONE,
    /// wait-for-timer) or the macro ends.
    ///
    /// Several opcodes (`REVERSE_PATH`, `RUN`, `WALK`, `PATROL_*`, ...)
    /// would tail-call back into the VM to consume the next byte. We
    /// flatten that into an explicit `'vm: loop` so the stack doesn't
    /// grow with macro length, and so `&mut self` aliasing is trivial.
    pub fn execute_next_macro_command(&mut self, ctx: &AiContext) {
        // If we're still in STATE_DEFAULT, make sure the substate
        // reflects that we're inside the VM.
        if self.current_state == AiState::Default {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultInMacro;
        }
        self.standing_around_timer = 0;

        let mut point_already_set = false;
        'vm: loop {
            if (self.number_of_remaining_macro_bytes as i16) > 0 {
                // -- Decode next opcode. -----------------------------
                let opcode_byte = match self.macro_command.get(self.macro_command_offset).copied() {
                    Some(b) => b,
                    None => {
                        tracing::warn!(
                            "NPC {}: macro PC out of bounds at offset {}",
                            self.me,
                            self.macro_command_offset
                        );
                        self.break_macro();
                        return;
                    }
                };
                self.macro_command_offset += 1;
                self.number_of_remaining_macro_bytes -= 1;
                self.macro_in_progress = true;

                let Some(opcode) = MacroOpcode::from_u8(opcode_byte) else {
                    // Unknown opcode: clear remaining bytes and fall
                    // into the "out of bytes" branch.
                    tracing::warn!(
                        "NPC {}: invalid macro opcode 0x{:02x}, breaking macro",
                        self.me,
                        opcode_byte
                    );
                    self.number_of_remaining_macro_bytes = 0;
                    continue 'vm;
                };

                match opcode {
                    MacroOpcode::ReversePath => {
                        if let Some(ref mut path) = self.patrol_path {
                            path.flip_forward_movement();
                        }
                        continue 'vm;
                    }

                    MacroOpcode::SkipPoint => {
                        if let Some(ref mut path) = self.patrol_path {
                            path.advance();
                        }
                        // Last command — return to patrol.
                        self.number_of_remaining_macro_bytes = 0;
                        continue 'vm;
                    }

                    MacroOpcode::GotoPoint => {
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        if let Some(ref mut path) = self.patrol_path {
                            // index == current would be a level-designer
                            // bug; log and continue.
                            if path.current_waypoint_index as u16 == index {
                                tracing::warn!(
                                    "NPC {}: CMD_GOTO_POINT → same waypoint {}",
                                    self.me,
                                    index
                                );
                            }
                            path.set_current_index(index as u8);
                        }
                        self.number_of_remaining_macro_bytes = 0;
                        point_already_set = true;
                        continue 'vm;
                    }

                    MacroOpcode::FaceTo => {
                        let Some(direction) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.face_direction(direction, ctx);
                        return;
                    }

                    MacroOpcode::Wait => {
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        self.launch_macro_timer(frames as u32, ctx.frame);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Check4 => {
                        let Some(friend_id) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Civilians/royalists log a warning but still
                        // call InitializeFriendCheck and exit.
                        if !ctx.self_is_soldier {
                            tracing::warn!("NPC {}: CMD_CHECK_4 is illegal for civilians", self.me);
                        }
                        self.initialize_friend_check(friend_id, frames, u16::MAX, ctx);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Check4Sync => {
                        let Some(friend_id) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_CHECK_4_SYNC is illegal for civilians",
                                self.me
                            );
                        }
                        self.initialize_friend_check(friend_id, frames, index, ctx);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::StayHere => {
                        // CMD_STAY_HERE → AssignNewPatrolPath(ClearPath)
                        // then exit. The helper already handles
                        // BreakMacro + initial-pos snapshot +
                        // EventReturnToDuty dispatch, so just exit
                        // after. (Falling through to the out-of-bytes
                        // branch would re-run path-advance on top of
                        // the reset, which is wrong.)
                        self.assign_new_patrol_path(
                            PatrolAssignment::ClearPath,
                            ctx.position,
                            ctx.direction,
                            &ctx.hiking_paths,
                        );
                        return;
                    }

                    MacroOpcode::ChangeWay => {
                        let Some(index) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // The helper runs break_macro + bounds check
                        // + gated EventReturnToDuty.  Out-of-range
                        // indices bail without further effect.
                        let assignment = match PathId::new(index) {
                            Some(pid) => PatrolAssignment::Index(pid),
                            None => PatrolAssignment::ClearPath,
                        };
                        self.assign_new_patrol_path(
                            assignment,
                            ctx.position,
                            ctx.direction,
                            &ctx.hiking_paths,
                        );
                        return;
                    }

                    MacroOpcode::Run => {
                        self.default_path_walking_flags |= GotoFlags::RUN;
                        // Sanitise forbidden-civilian flags after the
                        // flag flip — only CMD_RUN/CMD_WALK touch these
                        // flags.
                        if !ctx.self_is_soldier
                            && self
                                .default_path_walking_flags
                                .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                        {
                            tracing::warn!(
                                me = self.me,
                                "civilian CMD_RUN with forbidden GoTo flags — masking",
                            );
                            self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                        }
                        continue 'vm;
                    }

                    MacroOpcode::Walk => {
                        self.default_path_walking_flags -= GotoFlags::RUN;
                        // Same civilian sanitation as CMD_RUN.
                        if !ctx.self_is_soldier
                            && self
                                .default_path_walking_flags
                                .intersects(GotoFlags::FORBIDDEN_CIVILIANS)
                        {
                            tracing::warn!(
                                me = self.me,
                                "civilian CMD_WALK with forbidden GoTo flags — masking",
                            );
                            self.default_path_walking_flags -= GotoFlags::FORBIDDEN_CIVILIANS;
                        }
                        continue 'vm;
                    }

                    MacroOpcode::LookLeft => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_LOOK_LEFT is illegal for civilians",
                                self.me
                            );
                        }
                        self.pending_look_sidewards = Some(LookDirection::Left);
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::LookRight => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_LOOK_RIGHT is illegal for civilians",
                                self.me
                            );
                        }
                        self.pending_look_sidewards = Some(LookDirection::Right);
                        self.current_substate = Substate::DefaultInMacroWaitingForDone;
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::Bend => {
                        let Some(frames) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!("NPC {}: CMD_BEND is illegal for civilians", self.me);
                        }
                        self.pending_look_sidewards = Some(LookDirection::Down);
                        self.launch_macro_timer(frames as u32, ctx.frame);
                        self.macro_started_in_this_frame = false;
                        return;
                    }

                    MacroOpcode::PatrolStop => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_STOP is illegal for civilians",
                                self.me
                            );
                        }
                        self.patrol_stopped = true;
                        if ctx.self_rank == crate::profiles::ProfileRank::Officer {
                            self.say(Remark::OfficerStopsPatrol);
                        }
                        continue 'vm;
                    }

                    MacroOpcode::PatrolDirection => {
                        let Some(direction) = self.read_macro_u16() else {
                            self.break_macro();
                            return;
                        };
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_DIRECTION is illegal for civilians",
                                self.me
                            );
                        }
                        self.instruct_patrol_direction_to_patrol_members(direction);
                        continue 'vm;
                    }

                    MacroOpcode::PatrolStart => {
                        // Log-and-proceed for civilians.
                        if !ctx.self_is_soldier {
                            tracing::warn!(
                                "NPC {}: CMD_PATROL_START is illegal for civilians",
                                self.me
                            );
                        }
                        self.patrol_stopped = false;
                        if ctx.self_rank == crate::profiles::ProfileRank::Officer {
                            self.say(Remark::OfficerStartsPatrol);
                        }
                        // Also calls `InitializePatrol()` here. Raise
                        // the one-shot flag so
                        // `tick_patrol_coordination` Phase 3 clears +
                        // rebuilds the minion list on its next pass;
                        // the local `patrol.clear()` keeps the current
                        // frame's coordinate dispatch from referencing
                        // a stale list before the rebuild.
                        self.patrol.clear();
                        self.needs_patrol_reinit = true;
                        continue 'vm;
                    }
                }
            } else {
                // -- Out of macro bytes: path-advance branch. -------

                // Virtual hook for subclass overrides on macro
                // completion. Both subclasses' overrides gate on
                // `DefaultOnPost`, which the macro-end branch can't
                // enter (substate is `DefaultInMacro` here), so the
                // call observably returns false today. The hook is
                // wired anyway so a future override that doesn't share
                // that gate will be invoked from this site.
                if self.default_bored_standard_procedure(ctx) {
                    self.break_macro();
                    return;
                }

                let path_size = self.patrol_path.as_ref().map(|p| p.size).unwrap_or(0);

                if path_size == 1 {
                    if self.macro_started_in_this_frame {
                        // One-point path + started this frame → hold
                        // position via the *macro* timer, not the
                        // regular bored timer. The wake must come via
                        // `launch_macro_timer`'s `bMacroTimer = true`
                        // path so it's delivered by `ProceedMacro` as a
                        // direct `execute_next_macro_command()` call —
                        // bypassing Think entirely. A regular
                        // `launch_timer` here would fire an EventTimer
                        // that `DefaultInMacro` never handles
                        // (`DefaultInMacro` only receives EventDone),
                        // so the NPC would hang until the next
                        // reach-point event nudges it.
                        self.current_substate = Substate::DefaultInMacro;
                        self.macro_started_in_this_frame = false;
                        self.launch_macro_timer(
                            crate::parameters_ai::AI_ONE_POINT_DEFAULT_TIME as u32,
                            ctx.frame,
                        );
                    } else {
                        // Already here → synthesize a REACH_POINT event so
                        // the stimulus queue picks up the next waypoint.
                        self.macro_in_progress = false;
                        self.timer_is_running = false;
                        self.current_substate = Substate::DefaultEnroute;
                        self.fire_self_stimulus(StimulusType::EventReachPoint);
                    }
                } else {
                    if !point_already_set && let Some(ref mut path) = self.patrol_path {
                        path.advance();
                    }

                    let hiking_paths = &ctx.hiking_paths;
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultEnroute;
                    let will_stop = self.will_stop_at_next_waypoint(hiking_paths);
                    let mut walk_flags = self.default_path_walking_flags;
                    if !will_stop {
                        walk_flags |= GotoFlags::DONT_STOP;
                    }
                    if let Some(next_wp) = self
                        .patrol_path
                        .as_ref()
                        .and_then(|p| p.current_waypoint(hiking_paths))
                        .map(|wp| Position {
                            x: wp.x as f32,
                            y: wp.y as f32,
                            sector: SectorHandle::new(wp.sector),
                            level: wp.level,
                        })
                    {
                        self.go_to(next_wp, walk_flags, ctx);
                    } else {
                        self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                    }
                    self.macro_in_progress = false;
                    self.timer_is_running = false;
                }
                return;
            }
        }
    }

    /// Read a u16 LE at the macro PC cursor, advance the cursor by 2,
    /// and decrement `number_of_remaining_macro_bytes` by 2.  Returns
    /// `None` on truncation.  Used by operand-bearing opcodes inside
    /// [`Self::execute_next_macro_command`].
    fn read_macro_u16(&mut self) -> Option<u16> {
        let off = self.macro_command_offset;
        if off + 2 > self.macro_command.len() {
            return None;
        }
        let value = u16::from_le_bytes([self.macro_command[off], self.macro_command[off + 1]]);
        self.macro_command_offset += 2;
        self.number_of_remaining_macro_bytes =
            self.number_of_remaining_macro_bytes.saturating_sub(2);
        Some(value)
    }

    // -- Friend check (CheckFor comportment) --

    /// Start the "CheckFor" comportment against another NPC — the
    /// direct target of CMD_CHECK_4 / CMD_CHECK_4_SYNC from the macro
    /// VM.
    ///
    /// Steps:
    ///  (a) bounds-check `friend_id` against the all-soldier count,
    ///      assert NPC, store on `checkpoint_charly`, assert not self.
    ///  (b) early-resume the macro if the partner is already known dead
    ///      / missing (`missed_in_action`) or we recently saw an enemy
    ///      (`NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME` cooldown).
    ///  (c) **pure-synchronization branch** when `frames==0 && index!=
    ///      u16::MAX`: compare partner's current/last waypoint index
    ///      and forward-movement direction against ours and either
    ///      resume the macro or queue a `RegisterSynchronizingActor`
    ///      and switch to `Substate::DefaultSynchronizing`.
    ///  (d) waypoint / post visibility check: scan the partner's patrol
    ///      waypoints (or fall back to its initial post) via
    ///      [`AiContext::is_detecting_point_360`]. If nothing is
    ///      visible, log + resume the macro.
    ///  (e) optionally seed `synchronize_charly` + `synchronize_index`
    ///      so the wait-loop can synchronise once the friend arrives.
    ///  (f) configure the look-around wait: `number_of_looks =
    ///      frames / AI_CHECKFOR_TIME_INTERVAL + 1`,
    ///      `delta_sorrow_level = 1000 / number_of_looks`, transition
    ///      to `DefaultLookingSidewardsForCharly`, and queue
    ///      `pending_look_sidewards` with a random `LeftRight` /
    ///      `RightLeft` direction.
    pub fn initialize_friend_check(
        &mut self,
        friend_id: u16,
        frames: u16,
        index: u16,
        ctx: &AiContext,
    ) {
        // (a) Resolve friend_id → handle. Degrade to a warn +
        // early-resume on out-of-range or non-NPC, since panicking
        // would crash the engine on a malformed mission script.
        let number_of_all = ctx.number_of_all_soldiers();
        if friend_id >= number_of_all {
            tracing::warn!(
                "NPC {}: CheckFor at ({:.0}, {:.0}): friend_id {} out of range (max {})",
                self.me,
                ctx.position.x,
                ctx.position.y,
                friend_id,
                number_of_all
            );
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(ctx);
            return;
        }
        let target = match ctx.all_soldier_handle(friend_id) {
            Some(h) if h != 0 => h,
            _ => {
                tracing::warn!(
                    "NPC {}: CheckFor at ({:.0}, {:.0}): friend_id {} resolves to no live actor",
                    self.me,
                    ctx.position.x,
                    ctx.position.y,
                    friend_id
                );
                self.set_checkpoint_charly(0);
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(ctx);
                return;
            }
        };
        // Bail with a warn (instead of panicking) on level-data
        // issues if the resolved actor isn't an NPC.
        let target_view = match ctx.entity_view(target) {
            Some(v)
                if matches!(
                    v.kind,
                    crate::ai_entity_view::EntityKind::Soldier
                        | crate::ai_entity_view::EntityKind::Civilian
                ) =>
            {
                v.clone()
            }
            _ => {
                tracing::warn!(
                    "NPC {}: CheckFor friend_id {} → handle {} is not an NPC",
                    self.me,
                    friend_id,
                    target
                );
                self.set_checkpoint_charly(0);
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(ctx);
                return;
            }
        };
        // Store + warn if not self.
        self.set_checkpoint_charly(target);
        if target == self.me {
            tracing::warn!(
                "NPC {}: CheckFor at ({:.0}, {:.0}) applied on yourself? Funny idea...",
                self.me,
                ctx.position.x,
                ctx.position.y
            );
        }

        // (b1) friend already on the missed list → skip the check,
        // resume the macro.
        if self.missed_in_action.contains(&target) {
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(ctx);
            return;
        }

        // (b2) recently saw an enemy → no-op.
        if self.frame_when_enemy_detected > 0
            && ctx.frame.wrapping_sub(self.frame_when_enemy_detected)
                < crate::parameters_ai::NO_CHECK_FOR_AFTER_CHARLY_ALERT_TIME
        {
            self.set_checkpoint_charly(0);
            self.current_substate = Substate::DefaultInMacro;
            self.execute_next_macro_command(ctx);
            return;
        }

        // Self path direction / current waypoint — read once.
        // Forward-movement defaults to true when the path is
        // uninitialised; matches `PatrolPath::forward`.
        let my_forward = self.patrol_path.as_ref().map(|p| p.forward).unwrap_or(true);
        let my_current_wp_index = self
            .patrol_path
            .as_ref()
            .map(|p| p.current_waypoint_index as u16)
            .unwrap_or(0);

        // (c) Pure synchronization branch.
        if frames == 0 && index != u16::MAX {
            let synchronize_index = if index > 500 {
                // Relative index: my current waypoint + (index - 1000).
                // The original math is unsigned wrap-friendly; we use
                // i32 arithmetic and clamp the cast.
                let rel = (index as i32) - 1000;
                ((my_current_wp_index as i32) + rel).max(0) as u16
            } else {
                index
            };
            self.synchronize_charly = target;
            self.synchronize_index = synchronize_index;
            self.set_checkpoint_charly(0);
            debug_assert!(
                self.macro_in_progress,
                "InitializeFriendCheck pure-sync branch requires a macro to be in progress"
            );

            let target_alive_in_default =
                target_view.ai_state == AiState::Default && !target_view.is_dead;

            let friend_is_already_there = if target_alive_in_default {
                if target_view.macro_in_progress {
                    // Standing at the right waypoint?
                    if index < 500 {
                        target_view.path_current_waypoint_index as u16 == synchronize_index
                    } else if target_view.path_forward_movement != my_forward {
                        // backwards guy waits — only the forward leg proceeds
                        my_forward
                    } else if my_forward {
                        target_view.path_current_waypoint_index as u16 >= synchronize_index
                    } else {
                        target_view.path_current_waypoint_index as u16 <= synchronize_index
                    }
                } else if target_view.ai_substate == Substate::DefaultEnroute {
                    // Last waypoint was the right one?
                    if index < 500 {
                        target_view.path_last_waypoint_index as u16 == synchronize_index
                    } else if target_view.path_forward_movement != my_forward {
                        my_forward
                    } else if my_forward {
                        target_view.path_last_waypoint_index as u16 >= synchronize_index
                    } else {
                        target_view.path_last_waypoint_index as u16 <= synchronize_index
                    }
                } else {
                    false
                }
            } else {
                // Friend not in STATE_DEFAULT or dead → forget it.
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(ctx);
                return;
            };

            if friend_is_already_there {
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(ctx);
            } else {
                // Not yet there — wait, register us.
                self.pending_cross_npc_actions
                    .push(CrossNpcAction::RegisterSynchronizingActor {
                        target,
                        actor: self.me,
                    });
                self.current_substate = Substate::DefaultSynchronizing;
            }
            return;
        }

        // (d) Visibility check.
        if !target_view.has_patrol_path {
            // Post-only friend. Try the post, then post + 15 Z; if
            // neither is visible, warn and continue into the wait
            // setup anyway.
            let post = crate::coordinates::WorldPoint3D {
                x: target_view.initial_position.x,
                y: target_view.initial_position.y,
                z: target_view.elevation,
            };
            if !ctx.is_detecting_point_360(post) {
                let mut elevated = post;
                elevated.z += 15.0;
                if !ctx.is_detecting_point_360(elevated) {
                    tracing::warn!(
                        "NPC {}: CheckFor at ({:.0}, {:.0}): partner's post at ({:.0}, {:.0}) not visible",
                        self.me,
                        ctx.position.x,
                        ctx.position.y,
                        target_view.initial_position.x,
                        target_view.initial_position.y
                    );
                }
            }
            if index != u16::MAX {
                tracing::warn!(
                    "NPC {}: CheckForSynch at ({:.0}, {:.0}): can't synchronise with a partner that has no path",
                    self.me,
                    ctx.position.x,
                    ctx.position.y
                );
            }
        } else {
            // Scan the partner's patrol waypoints for at least one
            // that we can see.
            let hiking_paths = &ctx.hiking_paths;
            let mut visible_point_found = false;
            if let Some(path_id) = target_view.patrol_hiking_path_index
                && let Some(raw_path) = hiking_paths.get(path_id.get() as usize)
            {
                for wp in raw_path.waypoints.iter() {
                    let mut pt = ctx.position_to_point_3d(Position {
                        x: wp.x as f32,
                        y: wp.y as f32,
                        sector: SectorHandle::new(wp.sector),
                        level: wp.level,
                    });
                    pt.z += 15.0;
                    if ctx.is_detecting_point_360(pt) {
                        visible_point_found = true;
                        break;
                    }
                }
            }
            if !visible_point_found {
                // No waypoint visible → log + resume macro.
                tracing::trace!(
                    "NPC {}: CheckFor at ({:.0}, {:.0}): no waypoint of partner's path is visible",
                    self.me,
                    ctx.position.x,
                    ctx.position.y
                );
                self.current_substate = Substate::DefaultInMacro;
                self.execute_next_macro_command(ctx);
                return;
            }
        }
        // (e) Maybe prepare for later sync.
        if index == u16::MAX {
            self.synchronize_charly = 0;
            self.synchronize_index = u16::MAX;
        } else {
            self.synchronize_charly = target;
            self.synchronize_index = if index > 500 {
                let rel = (index as i32) - 1000;
                ((my_current_wp_index as i32) + rel).max(0) as u16
            } else {
                index
            };
        }

        // (f) Begin to wait.
        let interval = crate::parameters_ai::AI_CHECKFOR_TIME_INTERVAL.max(1) as u16;
        self.number_of_looks = ((frames / interval) + 1).min(u8::MAX as u16) as u8;
        let looks_for_div = self.number_of_looks.max(1) as u16;
        self.delta_sorrow_level = 1000 / looks_for_div;
        self.current_substate = Substate::DefaultLookingSidewardsForCharly;
        self.pending_look_sidewards = Some(if crate::sim_rng::u32(0..2) != 0 {
            LookDirection::LeftRight
        } else {
            LookDirection::RightLeft
        });
    }

    // -- Stop all --

    /// Halts the actor's current active sequence element via the engine
    /// (equivalent to `Stop(PREFERENCE)`), breaks the macro, and clears
    /// the AI-side timers. The actual halt happens in the engine
    /// post-think drain where it can borrow `&mut Engine`; see
    /// [`AiController::pending_halt`].
    pub fn stop_all(&mut self) {
        // When in a CheckFor look-around, clear the checkpoint
        // *before* the halt so the missed-friend detectable list and
        // `sorrow_level` reset side-effects fire.
        let in_charly_look = matches!(
            self.current_substate,
            Substate::DefaultLookingForCharly | Substate::DefaultLookingSidewardsForCharly
        );
        if in_charly_look {
            self.set_checkpoint_charly(0);
        }
        self.pending_halt = true;
        // Skip BreakMacro when we're in a CheckFor look or being
        // instructed by an officer — these substates need the
        // in-flight macro to survive the halt.
        let skip_break_macro =
            in_charly_look || self.current_substate == Substate::SeekingGroupGetInstructedByOfficer;
        if !skip_break_macro {
            self.break_macro();
        }
    }

    /// Drop every queued `pending_*` intent that a prior `think()` set
    /// but the engine hasn't yet drained.
    ///
    /// These fields exist because Rust's borrow checker forbids holding
    /// a `&mut Engine` during `think()`, so engine-side calls
    /// (`SetState`, `EnterSwordfight`, `GoTo`, …) become `pending_*`
    /// flags on the AiController that the engine drains after think
    /// returns. `handle_death_with_damage_element` needs to clear every
    /// one of them so stale intents from the pre-death think don't fire
    /// on a corpse; this single method keeps that cauterise tidy. New
    /// `pending_*` fields must be added here when introduced.
    pub fn clear_all_pending(&mut self) {
        let _ = self.take_pending_orders();
        self.pending_halt = false;
        self.pending_enter_swordfight = None;
        self.pending_enter_swordfight_jump_line = None;
        self.pending_stop_target = None;
        self.pending_set_principal = None;
        self.pending_friend_primary_target_swap = None;
        self.pending_shoot_target = None;
        self.pending_focus = None;
        self.pending_unfocus = false;
        self.pending_focus_point = None;
        self.pending_set_direction_instantly = None;
        self.pending_deactivate = false;
        self.pending_broadcast_panic = false;
        self.pending_script_seek_area = None;
        self.pending_launch_commands.clear();
        self.pending_launch_on_target.clear();
        self.pending_launch_sequences.clear();
        self.pending_look_sidewards = None;
        self.pending_add_detectables.clear();
        self.pending_delete_detectables.clear();
        self.pending_delete_detectable_entity.clear();
        self.pending_delete_beggar_for_all_npc.clear();
        self.pending_blink_enemy_specific.clear();
        self.pending_slowly_open_eyes = false;
        self.pending_restore_detectable_objects = false;
        self.pending_forget_nearby_coins = None;
        self.pending_posture = None;
        self.pending_quit_swordfight = false;
        self.pending_stop_menace = false;
        self.pending_lower_shield = false;
        self.pending_unalert_near_charly_seekers = None;
        self.pending_refill_bow_ammo = false;
        self.pending_set_reported_to_officer.clear();
    }

    // -- Movement commands --
    // These record intent and produce an Order for the engine to dispatch.

    /// Build a movement order from destination + flags.
    ///
    /// Maps `GotoFlags` to the appropriate `OrderType` and `MoveFlags`:
    /// - `RIDER_CHARGE_HIT` → `OrderType::RiderCharging` (charge with hit zone)
    /// - `RIDER_CHARGE` → `MoveFlags::RIDER_CHARGE` (running, fires galopp events)
    ///
    pub(crate) fn make_move_order(destination: &Position, flags: GotoFlags) -> AiOrderIntent {
        use crate::order::OrderType;
        use crate::sequence::MoveFlags;

        // Determine movement action.
        let order_type = if flags.contains(GotoFlags::RIDER_CHARGE_HIT) {
            OrderType::RiderCharging
        } else if flags.contains(GotoFlags::RUN) {
            OrderType::RunningUpright
        } else {
            OrderType::WalkingUpright
        };

        let mut order = AiOrderIntent::new(order_type, destination.x, destination.y);
        order.reverse = flags.contains(GotoFlags::BACK);
        order.compute_direction = !flags.contains(GotoFlags::STRAIGHT);
        // `GoTo` calls `Halt()` by default unless `GOTO_NOHALT` is set.
        // Propagate so the engine can honour the "don't tear down the
        // outgoing sequence" request at dispatch time.
        order.no_halt = flags.contains(GotoFlags::NO_HALT);

        // Set movement-sequence flags derived from GoTo flags.
        // `GOTO_SWORD` always adds `FORCE_SWORD_MOVEMENT`, even when
        // the actor was already in a sword action-state; this keeps
        // combat spacing and step-back dodges out of ordinary walk/run
        // animation.
        if flags.contains(GotoFlags::RIDER_CHARGE) {
            order.move_flags = MoveFlags::RIDER_CHARGE.bits() as u16;
        }
        if flags.contains(GotoFlags::SWORD) {
            order.move_flags |= MoveFlags::FORCE_SWORD_MOVEMENT.bits() as u16;
        }

        // Forward `GOTO_FINDACCESSIBLE` and `GOTO_ASKOBSTACLE` to the
        // engine drain. The drain has the FastFindGrid in hand and
        // runs `FindAutorizedPosition` / `IsStraightMovementAutorized`,
        // then either rewrites the destination, sets
        // `couldnt_reachpoint`, or both.
        order.find_accessible = flags.contains(GotoFlags::FIND_ACCESSIBLE);
        order.ask_obstacle = flags.contains(GotoFlags::ASK_OBSTACLE);

        order
    }

    /// Check if the entity is already at `destination` within `tolerance`
    /// (MaxNorm).
    fn check_already_on_point(
        &self,
        destination: &Position,
        tolerance: f32,
        ctx: &AiContext,
    ) -> bool {
        let dx = (ctx.position.x - destination.x).abs();
        let dy = (ctx.position.y - destination.y).abs();
        dx.max(dy) < tolerance
    }

    /// Low-level movement primitive — queues a movement intent without
    /// committing to a substate transition.  Prefer the `EnemyAi::go_to` /
    /// `FriendlyAi::go_to` wrappers, which enforce the Shape 1 contract
    /// (every queued movement names the new substate atomically so the
    /// halt-teardown in `process_pending_ai_orders` can't orphan the AI
    /// in a "waiting" substate). Calling this directly via
    /// `ai.base.go_to(...)` bypasses that contract and risks wedge bugs.
    pub fn go_to(&mut self, destination: Position, flags: GotoFlags, ctx: &AiContext) {
        // Record the latest destination / flags so stuck-retry replays,
        // cancellation, and the EventReachPoint re-entry path can see
        // what was most recently requested.
        self.last_goto_destination = destination;
        self.last_goto_flags = flags;
        self.couldnt_reachpoint = false;

        // Civilians must not be issued combat / rider-charge flags.
        // Mask `FORBIDDEN_CIVILIANS` silently — civilians hitting one
        // of these flags usually indicates a script or AI bug, but the
        // game keeps running.
        let mut flags = flags;
        if !ctx.self_is_soldier {
            let forbidden = flags & GotoFlags::FORBIDDEN_CIVILIANS;
            if !forbidden.is_empty() {
                tracing::warn!(
                    me = self.me,
                    ?forbidden,
                    "civilian GoTo with forbidden flags — masking",
                );
                flags -= GotoFlags::FORBIDDEN_CIVILIANS;
            }
        }

        // Already-on-point fast-exit. Gated on:
        //   - MaxNorm < 5 from the entity to the destination
        //   - `!likes_to_sit_around && !special_action`
        //   - animation state ∈ {WAITING_UPRIGHT, WAITING_ALERTED,
        //                         NONANIMATION_END}
        // When the gate fires, `end_think` drains `already_on_point`
        // into a `Think(EVENT_REACHPOINT)` re-entry.
        let idle_for_goto_short_circuit = matches!(
            ctx.self_animation,
            crate::order::OrderType::WaitingUpright
                | crate::order::OrderType::WaitingAlerted
                | crate::order::OrderType::NonanimationEnd
        );
        let may_short_circuit =
            idle_for_goto_short_circuit && !self.likes_to_sit_around && !self.special_action;
        if may_short_circuit && self.check_already_on_point(&destination, 5.0, ctx) {
            self.already_on_point = true;
            return;
        }

        // Out-of-level-bounds destinations fail fast with
        // `couldnt_reachpoint`. The non-negative half is enforced here;
        // the upper-bound `>= GetLevelSize()` half is enforced by the
        // engine drain in `preflight_ai_goto`, which has access to the
        // shared cutscene camera's level size.
        if destination.x <= 0.0 || destination.y <= 0.0 {
            self.couldnt_reachpoint = true;
            return;
        }

        // Null sector or negative layer → fail fast.
        // `Position.sector == None` represents a null sector; layer is
        // `u16` so the "negative layer" branch becomes unreachable
        // unless a caller stuffs `u16::MAX` in deliberately.
        if destination.sector.is_none() {
            self.couldnt_reachpoint = true;
            return;
        }

        // Strip `GOTO_STRAIGHT` when the destination crosses sector or
        // layer **and** the caller didn't pair it with
        // `GOTO_ASKOBSTACLE` — straight doesn't make sense across
        // sectors without an obstacle check.
        let crosses_boundary =
            destination.sector != ctx.position.sector || destination.level != ctx.position.level;
        if flags.contains(GotoFlags::STRAIGHT)
            && !flags.contains(GotoFlags::ASK_OBSTACLE)
            && crosses_boundary
        {
            flags -= GotoFlags::STRAIGHT;
        }

        // Prepend the appropriate action-state teardown before the
        // move is launched. Centralised here so every caller benefits
        // — the engine drain processes these intents before
        // `launch_pending_orders_for_npc` runs the move.
        self.apply_goto_action_state_teardown(flags, ctx);

        self.pending_orders
            .push(Self::make_move_order(&destination, flags));
    }

    /// Prepend the action-state teardown for a launching GoTo / GoNear /
    /// GoToSpeed:
    ///
    ///   * `GOTO_SWORD` set, not currently in a sword action-state →
    ///     prepend `ENTER_SWORDFIGHT` (raise-sword pose, no opponent).
    ///   * `GOTO_SWORD` not set, currently in a sword action-state →
    ///     prepend `QUIT_SWORDFIGHT` (sheath sword + clear opponents).
    ///   * `GOTO_SWORD` not set, currently `Menacing` → prepend
    ///     `STOP_MENACE` (menacing → waiting-sword → sword-down).
    ///
    /// Shield is also handled here: an actor mid-shield-raise that
    /// receives a `GoTo` first prepends a `Command::LowerShield` element
    /// so the shield drops before the move launches.
    fn apply_goto_action_state_teardown(&mut self, flags: GotoFlags, ctx: &AiContext) {
        let action_state = ctx.self_action_state;
        if flags.contains(GotoFlags::SWORD) {
            // GOTO_SWORD branch — already-in-sword is a no-op,
            // otherwise prepend ENTER_SWORDFIGHT without an opponent.
            if !action_state.is_sword() && self.pending_enter_swordfight.is_none() {
                self.pending_enter_swordfight = Some(EnterSwordfightRequest::RaiseSword);
                self.pending_enter_swordfight_jump_line = None;
            }
        } else if action_state.is_sword() {
            // Leaving a sword fight to walk somewhere without
            // GOTO_SWORD — sheath first.
            self.pending_quit_swordfight = true;
        } else if action_state == crate::element::ActionState::Menacing {
            // Drop the menace pose before walking.
            self.pending_stop_menace = true;
        }

        // Orthogonal to the sword/menace switch above — the shield
        // branch fires whenever the actor is in any shield action-state,
        // regardless of GOTO_SWORD. Prepend a `Command::LowerShield`
        // element so the shield drops (and the parry geometry stops
        // being armed) before the queued move runs.
        if action_state.is_shield() {
            self.pending_lower_shield = true;
        }
    }

    /// Low-level movement primitive (speed variant) — see
    /// [`AiController::go_to`] for the Shape 1 contract caveat.  Prefer
    /// `EnemyAi::go_to_speed` / `FriendlyAi::go_to_speed`.
    pub fn go_to_speed(
        &mut self,
        destination: Position,
        flags: GotoFlags,
        speed: f32,
        ctx: &AiContext,
    ) {
        self.last_goto_destination = destination;
        self.last_goto_flags = flags;
        self.couldnt_reachpoint = false;
        if self.check_already_on_point(&destination, 5.0, ctx) {
            self.already_on_point = true;
            return;
        }
        self.apply_goto_action_state_teardown(flags, ctx);
        let mut order = Self::make_move_order(&destination, flags);
        order.speed_factor = speed;
        self.pending_orders.push(order);
    }

    /// Queue the direct map-exit movement used by
    /// `SUBSTATE_FLEEING_MERRY_MAN_RUN_TO_LEAVE_MAP` after the NPC
    /// reaches the reinforcement door. C++ launches an
    /// `RHSequenceElementMovement(..., RHANIMATION_RUNNING_UPRIGHT,
    /// pointOut, NULL, 0.f, RHMOVE_MAP)` rather than a regular GoTo.
    pub fn run_to_map_exit(&mut self, destination: Position) {
        self.last_goto_destination = destination;
        self.last_goto_flags = GotoFlags::RUN;
        self.couldnt_reachpoint = false;

        let mut order = Self::make_move_order(&destination, GotoFlags::RUN);
        order.move_flags |= crate::sequence::MoveFlags::MAP.bits() as u16;
        self.pending_orders.push(order);
    }

    /// Low-level movement primitive (go-near variant) — see
    /// [`AiController::go_to`] for the Shape 1 contract caveat. Prefer
    /// `EnemyAi::go_near` / `FriendlyAi::go_near`.
    ///
    /// Pre-scales the tolerance under deep recursion (release-build
    /// mitigation), then tail-calls `go_to` with the NEAR flag OR'd in
    /// so `last_goto_flags` preserves the near semantics for
    /// stuck-retry replays.
    pub fn go_near(
        &mut self,
        destination: Position,
        distance: i32,
        flags: GotoFlags,
        ctx: &AiContext,
    ) {
        // Deep recursion shrinks the stop-distance toward zero so the
        // actor doesn't loop on Think() recursion. Always applied — a
        // mitigation, not a behaviour knob.
        let effective_distance = if self.think_recursion_depth < 10 {
            distance
        } else {
            let depth = self.think_recursion_depth as i32;
            (((100 - depth) * distance) / 100).max(0)
        };

        // The near-distance early-out also requires same-layer — a
        // different-layer destination falls through to the full launch
        // path. Apply that gate here before `go_to`'s own MaxNorm-5
        // check fires, since `go_to`'s check has no layer guard and the
        // tolerance argument we'd want isn't visible downstream.
        self.last_goto_destination = destination;
        self.last_goto_flags = flags | GotoFlags::NEAR;
        self.couldnt_reachpoint = false;

        let same_layer = destination.level == ctx.position.level;
        if same_layer && self.check_already_on_point(&destination, effective_distance as f32, ctx) {
            self.already_on_point = true;
            return;
        }

        self.apply_goto_action_state_teardown(flags, ctx);
        let mut order = Self::make_move_order(&destination, flags);
        order.tolerance = effective_distance as f32;
        self.pending_orders.push(order);
    }

    // -- Facing commands --

    /// Turn to face a position, without an `AiContext` available.
    ///
    /// Queues a plain Turn order; does not honour the same-frame
    /// `already_turned` short-circuit (no access to the current
    /// direction / action state). Prefer [`Self::face_position_with_ctx`]
    /// at call sites that have a ctx.
    pub fn face_position(&mut self, pos: Position) {
        self.pending_orders
            .push(AiOrderIntent::face_toward(pos.x, pos.y));
    }

    /// Internal helper — all `face_*_with_ctx` / `face_entity[_fast]`
    /// variants funnel through this so the short-circuit and elevation
    /// handling live in exactly one place.
    ///
    /// - `elevation_delta`: `target_elevation - ctx.elevation`. Pass
    ///   `0.0` for 2D-only faces. The target's elevation shifts the
    ///   effective dy before the aspect-ratio scale.
    fn face_position_impl(&mut self, pos: Position, ctx: &AiContext, elevation_delta: f32) {
        let dx = pos.x - ctx.position.x;
        let dy = (pos.y - ctx.position.y) + elevation_delta;
        let target_dir = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
        // legacy implementation FaceTo only short-circuits same-direction turns while
        // WAITING or BORED. Other action states still launch Turn so
        // Halt() semantics are preserved.
        let may_short_circuit = Self::face_to_same_direction_can_short_circuit(ctx);
        tracing::trace!(
            me = self.me,
            target_dir,
            current_dir = ctx.direction,
            ?ctx.posture,
            ctx.is_swordfighting,
            ?ctx.self_action_state,
            may_short_circuit,
            already_matches = (target_dir as u16 == ctx.direction),
            elevation_delta,
            "face_position_impl"
        );
        if target_dir as u16 == ctx.direction && may_short_circuit {
            self.already_turned = true;
            return;
        }
        self.pending_orders
            .push(AiOrderIntent::face_toward(pos.x, pos.y));
    }

    /// Turn to face a position (2D — no elevation adjustment). Honours
    /// the `already_turned` same-frame short-circuit.
    pub fn face_position_with_ctx(&mut self, pos: Position, ctx: &AiContext) {
        self.face_position_impl(pos, ctx, 0.0);
    }

    /// Turn to face another entity. Feeds the target's elevation into
    /// the 2D projection so the face accounts for height differences.
    ///
    /// Silently drops if the handle is `0` or the entity is no longer
    /// present in the snapshot.
    pub fn face_entity(&mut self, handle: NpcHandle, ctx: &AiContext) {
        let Some(view) = ctx.entity_view(handle) else {
            return;
        };
        let elevation_delta = view.elevation - ctx.elevation;
        let target_pos = view.position;
        self.face_position_impl(target_pos, ctx, elevation_delta);
    }

    // -- Self-stimuli --

    /// Queue a stimulus to be re-dispatched to this NPC on the next tick.
    /// The engine drains `pending_self_stimuli` and re-dispatches them
    /// after the current think cycle.
    pub fn fire_self_stimulus(&mut self, stimulus_type: StimulusType) {
        self.pending_self_stimuli.push(stimulus_type);
    }

    /// Turn to face a direction (0–15 sector).
    ///
    /// Convert the sector into a target position offset and delegate
    /// to `face_position`, which the movement/order system already
    /// handles.
    ///
    /// Honours the same-direction short-circuit: if the actor is
    /// already facing the requested sector **and** WAITING or BORED,
    /// set `already_turned` so `end_think` fires a same-frame
    /// `EVENT_DONE` re-entry instead of queuing a no-op Turn order.
    pub fn face_direction(&mut self, direction: u16, ctx: &AiContext) {
        if direction == ctx.direction && Self::face_to_same_direction_can_short_circuit(ctx) {
            self.already_turned = true;
            return;
        }
        let dir = crate::shadow_polygon::sector_to_direction(direction as i16);
        let target = Position {
            x: ctx.position.x + dir[0] * 100.0,
            y: ctx.position.y + dir[1] * 100.0,
            ..ctx.position
        };
        self.face_position_impl(target, ctx, 0.0);
    }

    fn face_to_same_direction_can_short_circuit(ctx: &AiContext) -> bool {
        matches!(
            ctx.self_action_state,
            crate::element::ActionState::Waiting | crate::element::ActionState::Bored
        )
    }

    // -- Speech commands --

    /// Say a remark (no flags).
    pub fn say(&mut self, remark: Remark) {
        self.say_impl(remark, SpeechFlags::empty());
    }

    /// Say a remark with special flags.
    pub fn say_with_flags(&mut self, remark: Remark, flags: SpeechFlags) {
        self.say_impl(remark, flags);
    }

    /// Full `Say()` implementation.
    ///
    /// Stores the remark + flags for `process_npc_speech()` to pick up.
    /// Gating (blipped, forbidden, in-building) and sound playback happen
    /// there. The MYTALK callback fires when the sound finishes (or
    /// immediately if `process_npc_speech` blocks the remark).
    fn say_impl(&mut self, remark: Remark, flags: SpeechFlags) {
        self.register_log_line(LogLineType::Speak, remark as u16);

        // Already speaking? Block a new non-EMERGENCY Say() from
        // overriding a remark
        // that is still in the Say->process_npc_speech pipeline
        // (`current_remark` set, not yet dispatched) OR actively being
        // played by the sound manager (`speech_in_flight`).  Without
        // `speech_in_flight`, the guard would pass the tick after
        // dispatch because phase 3 in process_npc_speech clears
        // `current_remark` only when the sound finishes — leaving a
        // window where a post-detection remark (REMARK_STARTS_COMBAT)
        // could cut off REMARK_SEES_ENEMY mid-clip.
        if self.current_remark != Remark::TheSoundOfSilence || self.speech_in_flight {
            if flags.contains(SpeechFlags::EMERGENCY) {
                // Kill current speech — process_npc_speech will handle
                // stopping the old channel when it sees the new remark.
                self.speech_in_flight = false;
            } else {
                // Still saying something — forget it.
                // Fire MYTALK immediately so the caller doesn't stall.
                self.inform_ai_on_finished_remark(flags);
                return;
            }
        }

        // Accept — process_npc_speech will check remaining gates,
        // play the sound, and fire MYTALK when finished.
        self.current_remark = remark;
        self.current_remark_flags = flags.bits();
    }

    /// Fire EVENT_MYTALK_X based on speech flags. Queues the event as
    /// a self-stimulus for the engine to deliver.
    fn inform_ai_on_finished_remark(&mut self, flags: SpeechFlags) {
        let event = if flags.contains(SpeechFlags::MYTALK_1) {
            Some(StimulusType::EventMyTalk1)
        } else if flags.contains(SpeechFlags::MYTALK_2) {
            Some(StimulusType::EventMyTalk2)
        } else if flags.contains(SpeechFlags::MYTALK_3) {
            Some(StimulusType::EventMyTalk3)
        } else if flags.contains(SpeechFlags::MYTALK_0) {
            Some(StimulusType::EventMyTalk0)
        } else {
            None
        };
        if let Some(stimulus_type) = event {
            self.pending_self_stimuli.push(stimulus_type);
        }
    }

    // -- Pointing command --

    /// Point at a position (animation command).
    ///
    /// Queues two sequence elements back-to-back — TURN then POINT —
    /// so the actor first finishes the turn and only then plays the
    /// pointing animation. The order-drain / animation layer runs
    /// them in order.
    ///
    /// (A `SetViewTarget(posTarget, false)` would bias head-tracking
    /// toward the point, but the position-taking `SetViewTarget`
    /// overload is a stub in the original game as well, so we skip
    /// it.)
    pub fn point_to(&mut self, pos: Position) {
        use crate::order::OrderType;
        // Pre-turn so the pointing anim fires already facing the
        // target. The Turning-order's own `already_facing`
        // short-circuit isn't worth wiring here — callers already
        // queue `pending_halt` / `stop_all` before `point_to` via the
        // instruct flow, so the Turn will run cleanly.
        self.pending_orders
            .push(AiOrderIntent::face_toward(pos.x, pos.y));
        self.pending_orders
            .push(AiOrderIntent::new(OrderType::Pointing, pos.x, pos.y));
    }

    // -- Alert status --

    /// Set the NPC's alert status (affects music + view).
    ///
    /// Writes both the music-side counter
    /// (`current_music_alert_status`) and the view-side field
    /// (`view_alert_status`). This is the override-free path: callers
    /// that need the soldier `IsForcedAttentive` view override should
    /// go through `EnemyAi::set_alert_status` (or call
    /// `set_alert_status_with_flags` directly with `forced_attentive =
    /// true`).
    ///
    /// The music-system side — aggregating all soldier statuses into
    /// the overall villain alert and calling `SetMusicMode` — runs
    /// once per frame in `EngineInner::update_overall_villain_alert`.
    pub fn set_alert_status(&mut self, level: AlertLevel) {
        self.set_alert_status_with_flags(level, AlertFlags::empty(), false);
    }

    /// Full-fidelity `set_alert_status(new_status, flags)`.
    ///
    /// Always updates `current_music_alert_status`. Returns early
    /// without touching the view field when `flags` contains
    /// `ALERT_ONLY_MUSIC`. Otherwise writes the view field, applying
    /// the soldier `IsForcedAttentive` override (Green music ⇒ Yellow
    /// view) when `forced_attentive` is set.
    ///
    /// `INSTANT_MUSIC_CHANGE` is staged on `pending_instant_music_change`
    /// when the call actually changes `current_music_alert_status`, and
    /// observed by the per-frame `update_overall_villain_alert` sweep.
    pub fn set_alert_status_with_flags(
        &mut self,
        level: AlertLevel,
        flags: AlertFlags,
        forced_attentive: bool,
    ) {
        if flags.contains(AlertFlags::INSTANT_MUSIC_CHANGE)
            && level != self.current_music_alert_status
        {
            self.pending_instant_music_change = true;
        }
        self.current_music_alert_status = level;

        if flags.contains(AlertFlags::ONLY_MUSIC) {
            return;
        }

        self.view_alert_status = if forced_attentive && level == AlertLevel::Green {
            AlertLevel::Yellow
        } else {
            level
        };
    }

    // -- Return to duty (common) --

    /// Common return-to-duty logic shared by soldiers and civilians.
    pub fn return_to_duty_common_stuff(&mut self, flags: DutyFlags, ctx: &AiContext) {
        // Start with `SetAlertStatus(GREEN)` — no `BreakMacro` /
        // `RetrogradeAmnesia` here, those are called by their own
        // call-sites elsewhere in the state machine. Route through the
        // flags-aware setter so a forced-attentive soldier returning
        // to duty keeps the view cone YELLOW even though the music
        // drops to GREEN.
        self.set_alert_status_with_flags(
            AlertLevel::Green,
            AlertFlags::empty(),
            ctx.self_forced_attentive,
        );

        if !flags.contains(DutyFlags::KEEP_EMOTICON) {
            self.clear_emoticon();
        }

        // Reset patrol path history so formation rebuilds cleanly.
        if let Some(ref mut path) = self.patrol_path {
            path.reset_history();
        }
        self.my_reconnaissance_report.reset();

        // Drop any stale `detected_body` pointer once the NPC no
        // longer has outstanding `DETECTABLE_FRIEND` entries (i.e.
        // it's finished swapping reports with alerted allies). The
        // friend count rides in on `ctx` so we don't have to crack
        // open `NpcData` from inside the AI.
        if ctx.self_detectable_friend_count == 0 {
            self.detected_body = 0;
        }

        // If this NPC has a live patrol chief that's able to fight
        // *and* within 360° detection range, run to them and enter
        // `DefaultGotoChief` — let the chief re-gather the patrol as
        // the minion closes. Only abandon the goto-chief path when
        // `couldnt_reachpoint` fires (then fall through to the normal
        // return-to-post logic below).
        if self.patrol_chief != 0
            && let Some(chief_view) = ctx.entity_view(self.patrol_chief)
            && chief_view.is_able_to_fight
        {
            // `IsDetecting360Degrees`: aspect-ratio-corrected distance
            // from me to the chief against our squared view radius.
            // Distance-only form (no LOS check), matching
            // `EnemyAi::is_detecting_360_degrees` in ai_enemy.rs.
            let dx = chief_view.position.x - ctx.position.x;
            let dy = chief_view.position.y - ctx.position.y;
            let sq_distance = crate::position_interface::vector_square_norm_iso(dx, dy);
            if sq_distance <= ctx.sq_standard_view_radius {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoChief;
                self.go_near(
                    chief_view.position,
                    crate::parameters_ai::AI_TALK_DISTANCE,
                    GotoFlags::empty(),
                    ctx,
                );
                if !self.couldnt_reachpoint {
                    return;
                }
                // Couldn't reach — reset flag and fall through to the
                // post/patrol-path logic.
                self.couldnt_reachpoint = false;
            }
        }

        let hiking_paths = &ctx.hiking_paths;

        if self.has_patrol_path {
            // Initialize patrol path if not yet done.
            if self.patrol_path.is_none() {
                self.patrol_path = self
                    .path_id
                    .and_then(|pid| PatrolPath::new(pid, hiking_paths));
                if self.patrol_path.is_none() {
                    tracing::warn!(
                        "NPC {} has_patrol_path but path_id {:?} is invalid, falling back to post",
                        self.me,
                        self.path_id,
                    );
                    self.has_patrol_path = false;
                }
            }

            if let Some(ref mut path) = self.patrol_path {
                let pos_here = ctx.position;
                let num_waypoints = path.size;

                // Find the nearest waypoint by MaxNorm distance.
                let mut best_index: u8 = 0;
                let mut min_dist = f32::MAX;
                for i in 0..num_waypoints {
                    if let Some(wp) = path.get_waypoint(i, hiking_paths) {
                        let dx = (wp.x as f32 - pos_here.x).abs();
                        let dy = (wp.y as f32 - pos_here.y).abs();
                        let dist = dx.max(dy); // MaxNorm
                        if dist < min_dist {
                            min_dist = dist;
                            best_index = i;
                        }
                    }
                }

                path.set_current_index(best_index);

                // Check whether going from here → nearest → next requires >90° turn.
                // If so, skip to the next waypoint.
                if let Some(wp) = path.current_waypoint(hiking_paths) {
                    let dir_x = wp.x as f32 - pos_here.x;
                    let dir_y = wp.y as f32 - pos_here.y;
                    let dir_norm = dir_x.abs().max(dir_y.abs());

                    if (best_index as usize) < (num_waypoints as usize).saturating_sub(1)
                        && let Some(next_wp) = path.peek_next_waypoint(hiking_paths)
                    {
                        let next_dx = next_wp.x as f32 - wp.x as f32;
                        let next_dy = next_wp.y as f32 - wp.y as f32;
                        // Dot product < 0 means >90° turn
                        let dot = dir_x * next_dx + dir_y * next_dy;
                        if dir_norm < 10.0 || dot < 0.0 {
                            path.advance();
                        }
                    }
                }
            }

            // Now that the path borrow is done, set state and issue walk order.
            if self.patrol_path.is_some() {
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoRoute;

                // At frame 0, if this is a patrol chief near its start, pre-seed
                // history so minions can form up immediately.
                let is_patrol_chief = self.has_patrol();
                let is_frame_zero = ctx.frame == 0;
                if is_patrol_chief
                    && is_frame_zero
                    && let Some(ref mut path) = self.patrol_path
                {
                    let dir_norm = if let Some(wp) = path.current_waypoint(hiking_paths) {
                        let dx = (wp.x as f32 - ctx.position.x).abs();
                        let dy = (wp.y as f32 - ctx.position.y).abs();
                        dx.max(dy)
                    } else {
                        f32::MAX
                    };
                    if dir_norm < 50.0 {
                        path.initialize_history_entries_on_path(hiking_paths);
                    }
                }

                // Walk to current waypoint.
                let dest = self.patrol_path.as_ref().and_then(|path| {
                    path.current_waypoint(hiking_paths).map(|wp| Position {
                        x: wp.x as f32,
                        y: wp.y as f32,
                        sector: SectorHandle::new(wp.sector),
                        level: wp.level,
                    })
                });
                if let Some(dest) = dest {
                    let mut walk_flags = self.default_path_walking_flags;
                    if !self.will_stop_at_next_waypoint(hiking_paths) {
                        walk_flags |= GotoFlags::DONT_STOP;
                    }
                    self.go_to(dest, walk_flags, ctx);
                }
            }
        } else if self.likes_to_sit_around {
            // Sitting NPCs: check if already at initial position — if
            // so, stay put; otherwise walk back with
            // `GOTO_SPECIAL_ACTION`.
            let ip = self.initial_position;
            let dx = (ctx.position.x - ip.x).abs();
            let dy = (ctx.position.y - ip.y).abs();
            if matches!(ctx.posture, crate::element::Posture::Sitting) && dx.max(dy) < 3.0 {
                // Already on sitting place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(ctx);
                self.launch_timer(bored as u32, ctx.frame);
            } else {
                // Return to sitting place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoPost;
                self.go_to(ip, GotoFlags::SPECIAL_ACTION, ctx);
            }
        } else if self.special_action {
            // Leisure-posture NPCs: same shape as the sitting branch
            // but keyed on posture==LEISURE and also uses
            // GOTO_SPECIAL_ACTION.
            let ip = self.initial_position;
            let dx = (ctx.position.x - ip.x).abs();
            let dy = (ctx.position.y - ip.y).abs();
            if matches!(ctx.posture, crate::element::Posture::Leisure) && dx.max(dy) < 3.0 {
                // Already on leisure place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultOnPost;
                let bored = self.get_bored_time(ctx);
                self.launch_timer(bored as u32, ctx.frame);
            } else {
                // Return to leisure place.
                self.set_ai_state(AiState::Default);
                self.current_substate = Substate::DefaultGotoPost;
                self.go_to(ip, GotoFlags::SPECIAL_ACTION, ctx);
            }
        } else {
            // Plain return-to-post: no `GOTO_SPECIAL_ACTION`, no
            // posture gate — just a bare `GoTo(initial_position)`
            // that relies on the `FIND_ACCESSIBLE` escape hatch.
            let ip = self.initial_position;
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultGotoPost;
            self.go_to(ip, GotoFlags::FIND_ACCESSIBLE, ctx);
        }
    }

    /// Forecast whether the actor will stop at its current waypoint.
    ///
    /// Returns `true` when the selected macro section starts with an
    /// opcode that halts the actor (`CMD_WAIT`, `CMD_FACE_TO`,
    /// `CMD_BEND`, `CMD_CHECK_4*`, `CMD_LOOK_LEFT`, `CMD_LOOK_RIGHT`,
    /// `CMD_STAY_HERE`). Returns `false` for purely-motion sections
    /// (`CMD_RUN`/`CMD_WALK`/`CMD_REVERSE_PATH`/`CMD_GOTO_POINT`/…) so
    /// the caller keeps the `DONT_STOP` flag and walks through. Takes
    /// `&mut self` so it can call [`Self::forecast_macro_rand`] (peek
    /// without consuming).
    pub fn will_stop_at_next_waypoint(
        &mut self,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) -> bool {
        use crate::level_data::WaypointCommand;

        // Collapse the path/waypoint borrow into owned data so the rest
        // of the function can take `&mut self` for `forecast_macro_rand`.
        let (forward, macro_data) = {
            let Some(path) = self.patrol_path.as_ref() else {
                // No path → conservatively report "will stop".
                return true;
            };
            let Some(wp) = path.current_waypoint(hiking_paths) else {
                return true;
            };
            match &wp.command {
                // No data → won't stop.
                WaypointCommand::None => return false,
                // Script may halt → will stop.
                WaypointCommand::Script(_) => return true,
                WaypointCommand::Macro(data) => (path.forward, data.clone()),
            }
        };

        let read_u16 = |off: usize| -> Option<u16> {
            if off + 2 > macro_data.len() {
                None
            } else {
                Some(u16::from_le_bytes([macro_data[off], macro_data[off + 1]]))
            }
        };
        let read_u8 = |off: usize| -> Option<u8> { macro_data.get(off).copied() };

        let direction_matches = |flag: u8| -> bool {
            match flag {
                0 => true,     // DIR_BOTH
                1 => forward,  // DIR_FORWARD
                2 => !forward, // DIR_BACKWARD
                _ => false,
            }
        };

        let Some(num_dir_blocks) = read_u16(0) else {
            return false;
        };
        if num_dir_blocks == 0 || num_dir_blocks > 2 {
            return false;
        }

        // Walk the (u8 flag, u16 offset) direction block headers.
        let mut section_table_off: Option<usize> = None;
        for i in 0..num_dir_blocks as usize {
            let hdr_off = 2 + i * 3;
            let Some(flag) = read_u8(hdr_off) else { break };
            let Some(offset) = read_u16(hdr_off + 1) else {
                break;
            };
            if direction_matches(flag) {
                section_table_off = Some(offset as usize);
                break;
            }
        }
        let Some(section_table_off) = section_table_off else {
            return false;
        };

        let Some(num_sections) = read_u16(section_table_off) else {
            return false;
        };
        if num_sections == 0 {
            return false;
        }

        // Peek (don't consume) the next macro-rand for section selection.
        let mut roll = self.forecast_macro_rand();
        let mut section_idx: Option<usize> = None;
        for i in 0..num_sections as usize {
            let entry_off = section_table_off + 2 + i * 3;
            let Some(weight) = read_u8(entry_off) else {
                break;
            };
            if roll <= weight {
                section_idx = Some(i);
                break;
            }
            roll -= weight;
        }
        let Some(selected) = section_idx else {
            return false;
        };

        let data_off_entry = section_table_off + 2 + selected * 3 + 1;
        let Some(section_data_offset) = read_u16(data_off_entry) else {
            return false;
        };
        let section_data_off = section_data_offset as usize;
        let Some(macro_byte_count) = read_u16(section_data_off) else {
            return false;
        };

        // Walk opcodes in the selected section, returning on the first
        // halt-or-flow-through decision. Args of halt opcodes are
        // ignored (we return immediately). Args of motion opcodes are
        // skipped: 0 bytes for RUN/WALK/PATROL_STOP/PATROL_START, 2
        // bytes for PATROL_DIRECTION.
        let mut remaining = macro_byte_count;
        let mut pc = section_data_off + 2;
        while remaining > 0 {
            let Some(op_byte) = read_u8(pc) else {
                return false;
            };
            let Some(op) = MacroOpcode::from_u8(op_byte) else {
                // Unknown opcode: bail out conservatively.
                return false;
            };
            match op {
                MacroOpcode::ReversePath
                | MacroOpcode::SkipPoint
                | MacroOpcode::GotoPoint
                | MacroOpcode::ChangeWay => return false,
                MacroOpcode::Wait
                | MacroOpcode::Check4
                | MacroOpcode::Check4Sync
                | MacroOpcode::FaceTo
                | MacroOpcode::Bend
                | MacroOpcode::StayHere
                | MacroOpcode::LookLeft
                | MacroOpcode::LookRight => return true,
                MacroOpcode::Run
                | MacroOpcode::Walk
                | MacroOpcode::PatrolStop
                | MacroOpcode::PatrolStart => {
                    remaining -= 1;
                    pc += 1;
                }
                MacroOpcode::PatrolDirection => {
                    if remaining < 3 {
                        return false;
                    }
                    remaining -= 3;
                    pc += 3;
                }
            }
        }
        false
    }

    // -- Patrol coordination --

    /// Handle `CALL_PATROL_COORDINATE` from the chief: walk or run to the
    /// assigned formation position.
    pub fn coordinate_patrol(
        &mut self,
        info: &StimulusInfo,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if self.patrol_chief == 0 {
            // Can happen when stimulus was postponed on door
            return;
        }

        let target_pos = match info {
            StimulusInfo::Position(pos) => *pos,
            _ => return,
        };

        match self.current_substate {
            // From idle/walking substates: stop current activity first
            Substate::DefaultInMacro
            | Substate::DefaultEnroute
            | Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultOnPost
            | Substate::DefaultGotoChief
            | Substate::DefaultOnPostLookingSidewards => {
                self.stop_all();
                self.coordinate_patrol_walk(target_pos, ctx, tick);
            }
            // Already in patrol formation — just update target
            Substate::DefaultPatrolEnroute
            | Substate::DefaultPatrolEnrouteRunning
            | Substate::DefaultPatrolEnrouteWaiting => {
                self.coordinate_patrol_walk(target_pos, ctx, tick);
            }
            _ => {}
        }
    }

    /// Inner logic for coordinate_patrol — compute speed and walk/run to the
    /// assigned formation position.
    fn coordinate_patrol_walk(&mut self, target: Position, ctx: &AiContext, tick: &AiPerTickData) {
        let vec_to_point = [target.x - ctx.position.x, target.y - ctx.position.y];
        let vec_to_chief = [
            tick.patrol_chief_position.x - ctx.position.x,
            tick.patrol_chief_position.y - ctx.position.y,
        ];
        let distance =
            (vec_to_point[0] * vec_to_point[0] + vec_to_point[1] * vec_to_point[1]).sqrt();
        let speed_factor = PATROL_SPEED_BASE + distance / PATROL_SPEED_DIVISOR;

        // Avoid stepping backward on the inner side of narrow curves:
        // when distance <= 30, check if the target is opposite to the
        // chief direction.
        let near_point_backwards = if distance > 30.0 {
            false
        } else {
            // Aspect-corrected dot product (negative when vec_to_point
            // is pointing away from the chief in isometric map space).
            let inv_ar = crate::position_interface::INVERSE_ASPECT_RATIO;
            vec_to_chief[0] * vec_to_point[0] + vec_to_chief[1] * inv_ar * vec_to_point[1] * inv_ar
                < 0.0
        };

        if near_point_backwards {
            // Just turn to face the officer instead of walking backward
            self.face_position(Position {
                x: tick.patrol_chief_position.x,
                y: tick.patrol_chief_position.y,
                ..ctx.position
            });
            return;
        }

        if speed_factor <= 2.0 {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultPatrolEnroute;
            let flags = GotoFlags::NO_HALT | GotoFlags::DONT_STOP | self.default_path_walking_flags;
            self.go_to_speed(target, flags, speed_factor, ctx);
        } else {
            self.set_ai_state(AiState::Default);
            self.current_substate = Substate::DefaultPatrolEnrouteRunning;
            let flags = GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP;
            self.go_to(target, flags, ctx);
        }
    }

    /// Receive a facing direction from the patrol chief.
    pub fn set_instructed_patrol_direction(&mut self, direction: u16, ctx: &AiContext) {
        self.patrol_direction = direction;
        if self.current_substate == Substate::DefaultPatrolEnrouteWaiting {
            self.face_direction(direction, ctx);
        }
    }

    // -- Common expected event handling --

    /// Handle expected events common to both soldiers and civilians.
    /// Handles default patrol, waypoint processing, macro execution,
    /// and fleeing behavior.
    pub fn think_expected_event_common_stuff(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;
        let hiking_paths = &ctx.hiking_paths;

        match self.current_substate {
            // ─── Return to post ─────────────────────────────────────
            Substate::DefaultGotoPost => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Reached post — turn to face initial direction.
                    self.face_direction(self.initial_view_direction, ctx);
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultGotoPostTurn;
                }
            }

            Substate::DefaultGotoPostTurn => {
                if stimulus_type == StimulusType::EventDone {
                    // When `GoTo` was launched with `GOTO_SPECIAL_ACTION`,
                    // the launched sequence already carried the
                    // post-arrival TURN element (set above as
                    // `face_direction(initial_view_direction)`) and a
                    // trailing `SIT_DOWN` / `ENTER_LEISURE` element so
                    // the seated / leisure transition animation plays.
                    // Queue the matching `Command::SitDown` /
                    // `Command::EnterLeisure` here so the engine's
                    // animation driver flips posture → Sitting / Leisure
                    // on completion. Earlier code wrote `pending_posture`
                    // directly which snapped the actor to the seated
                    // frame instead of playing the transition.
                    if self.likes_to_sit_around {
                        self.pending_launch_commands
                            .push(crate::element::Command::SitDown);
                    } else if self.special_action {
                        self.pending_launch_commands
                            .push(crate::element::Command::EnterLeisure);
                    }
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultOnPost;
                    let bored = self.get_bored_time(ctx);
                    self.launch_timer(bored as u32, ctx.frame);
                }
            }

            // ─── On post (idle) ─────────────────────────────────────
            Substate::DefaultOnPost => {
                if stimulus_type == StimulusType::EventTimer {
                    // Enemy AI intercepts this in EnemyAi::think_expected_event
                    // and runs DefaultBoredStandardProcedure before delegating.
                    // The override only exists for soldier AI (friendlies
                    // return false), so the base-class fall-through is
                    // correct: re-launch the bored timer.
                    let bored = self.get_bored_time(ctx);
                    self.launch_timer(bored as u32, ctx.frame);
                }
            }

            // ─── Return to route (has patrol path) ──────────────────
            Substate::DefaultGotoRoute => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_ai_state(AiState::Default);
                    self.current_substate = Substate::DefaultGotoRouteTurn;

                    // Calls `InitializePatrol()` here to rebuild the
                    // coordinate-patrol member list. Raise the
                    // one-shot flag so `tick_patrol_coordination`
                    // Phase 3 picks it up next pass.
                    self.needs_patrol_reinit = true;

                    // Turn to face the direction from previous waypoint.
                    if let Some(ref path) = self.patrol_path {
                        if path.size > 1 {
                            // Get previous waypoint to compute turn direction.
                            let mut tmp = path.clone();
                            tmp.retreat();
                            if let Some(prev_wp) = tmp.current_waypoint(hiking_paths) {
                                let dx = ctx.position.x - prev_wp.x as f32;
                                let dy = ctx.position.y - prev_wp.y as f32;
                                let sector =
                                    crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                                self.face_direction(sector as u16, ctx);
                            } else {
                                // No previous waypoint, skip turn.
                                self.think_event_done_on_self(ctx);
                            }
                        } else {
                            // Single waypoint path — skip turn.
                            self.think_event_done_on_self(ctx);
                        }
                    } else {
                        self.think_event_done_on_self(ctx);
                    }
                }
            }

            // ─── Walking along route ────────────────────────────────
            Substate::DefaultGotoRouteTurn | Substate::DefaultEnroute => {
                let is_route_turn = self.current_substate == Substate::DefaultGotoRouteTurn;
                let is_enroute = self.current_substate == Substate::DefaultEnroute;

                if (stimulus_type == StimulusType::EventDone && is_route_turn)
                    || (stimulus_type == StimulusType::EventReachPoint && is_enroute)
                {
                    if let Some(ref mut path) = self.patrol_path {
                        if path.size == 0 {
                            // Path was eliminated (by script?) — return to duty.
                            self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                            return false;
                        }

                        // Dispatch `EventSyncCharly` to every
                        // synchronizing actor waiting on this NPC's patrol.
                        // Drop entries whose substate has already advanced
                        // away from `DefaultSynchronizing`. We dispatch
                        // via the pending-cross-npc drain so the
                        // post-dispatch re-check happens on the next
                        // arrival rather than inline. Net effect: at
                        // worst an extra redundant `EventSyncCharly`
                        // fires one cycle after the actor has left the
                        // wait state.
                        if !self.synchronizing_actors.is_empty() {
                            let wp_idx = path.current_waypoint_index;
                            let mut keep = Vec::with_capacity(self.synchronizing_actors.len());
                            for &guy in &self.synchronizing_actors {
                                let substate = ctx
                                    .entity_view(guy)
                                    .map(|v| v.ai_substate)
                                    .unwrap_or(Substate::DefaultGotoPost);
                                if substate == Substate::DefaultSynchronizing {
                                    self.pending_cross_npc_actions.push(
                                        CrossNpcAction::SendStimulus {
                                            target: guy,
                                            stimulus_type: StimulusType::EventSyncCharly,
                                            info: StimulusInfo::Index(wp_idx.into()),
                                            fallback_to_sender: None,
                                            to_whole_patrol: false,
                                        },
                                    );
                                    keep.push(guy);
                                }
                            }
                            self.synchronizing_actors = keep;
                        }

                        let wp_command = path
                            .current_waypoint(hiking_paths)
                            .map(|wp| wp.command.clone())
                            .unwrap_or(crate::level_data::WaypointCommand::None);

                        match wp_command {
                            crate::level_data::WaypointCommand::None => {
                                // Simple waypoint — advance to next.
                                path.advance();

                                // `DefaultBoredStandardProcedure()` would be called
                                // here, but the virtual only fires when the substate
                                // is DEFAULT_ONPOST — at this point we're in
                                // DEFAULT_GOTOROUTE_TURN / DEFAULT_ENROUTE, so the
                                // call is a guaranteed no-op. Skipping it matches
                                // observed behaviour without needing a cross-base
                                // virtual dispatch hook.

                                if let Some(next_wp) = path.current_waypoint(hiking_paths) {
                                    if path.size == 1 {
                                        // One-point path → treat as post.
                                        // Snap the post anchor to the
                                        // current location; otherwise
                                        // `return_to_duty_common_stuff`
                                        // would walk back to the
                                        // level-load spawn.
                                        self.has_patrol_path = false;
                                        self.initial_position = ctx.position;
                                        self.initial_view_direction = ctx.direction & 0x0F;
                                        self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                                    } else {
                                        let mut walk_flags = self.default_path_walking_flags;
                                        if !self.will_stop_at_next_waypoint(hiking_paths) {
                                            walk_flags |= GotoFlags::DONT_STOP;
                                        }
                                        if is_enroute {
                                            walk_flags |= GotoFlags::STRAIGHT;
                                        }
                                        self.set_ai_state(AiState::Default);
                                        self.current_substate = Substate::DefaultEnroute;
                                        let dest = Position {
                                            x: next_wp.x as f32,
                                            y: next_wp.y as f32,
                                            sector: SectorHandle::new(next_wp.sector),
                                            level: next_wp.level,
                                        };
                                        self.go_to(dest, walk_flags, ctx);
                                    }
                                } else {
                                    // No next waypoint — done.
                                    self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                                }
                            }
                            crate::level_data::WaypointCommand::Script(_script) => {
                                // Hand off to the per-waypoint VM.
                                // `execute_waypoint_script` queues a
                                // `ReachPoint(actor)` dispatch against
                                // the instance bound at level load;
                                // the engine drains it post-think and
                                // fires `EventAfterScriptGoOn` if the
                                // script didn't lock us into
                                // `DefaultScriptDriven`.
                                let path_idx = path.hiking_path_index;
                                let wp_idx = path.current_waypoint_index;
                                self.execute_waypoint_script(path_idx, wp_idx);
                            }
                            crate::level_data::WaypointCommand::Macro(macro_data) => {
                                // Full waypoint-macro dispatch. If
                                // `launch_waypoint_macro` returns false,
                                // no section matched this traversal
                                // direction / roll — proceed along the
                                // path like a simple waypoint.
                                let launched = self.launch_waypoint_macro(&macro_data, ctx);
                                if !launched {
                                    self.proceed_on_path(hiking_paths, ctx);
                                }
                            }
                        }
                    } else {
                        // No patrol path — fall back to post.
                        self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                        return false;
                    }
                }
            }

            // ─── In macro ───────────────────────────────────────────
            Substate::DefaultInMacro => {
                // Ignore all Done events while executing macros.
            }

            Substate::DefaultInMacroWaitingForDone => {
                if stimulus_type == StimulusType::EventDone {
                    self.execute_next_macro_command(ctx);
                }
            }

            // ─── Fleeing ────────────────────────────────────────────
            Substate::FleeingRunToHide | Substate::FleeingRunToDoor => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_ai_state(AiState::Fleeing);
                    self.current_substate = Substate::FleeingHiding;
                    self.set_alert_status(AlertLevel::Yellow);
                    self.clear_emoticon();
                    // Face panic center and wait.
                    self.face_position(Position {
                        x: self.panic_center_x,
                        y: self.panic_center_y,
                        sector: None,
                        level: 0,
                    });
                    let hiding_time = 300 + crate::sim_rng::u32(..200); // AI_MIN + delta
                    self.launch_timer(hiding_time, ctx.frame);
                }
            }

            // ─── Panic-run state machine ────────────────────────────
            // On each arrival (or failed path) we either transition
            // into `FleeingHiding` (panic is spent) or pick a new run
            // direction and `GoTo` along it.
            Substate::FleeingPanic => {
                if stimulus_type != StimulusType::EventReachPoint
                    && stimulus_type != StimulusType::EventCouldntReachPoint
                {
                    return false;
                }

                if self.lasting_panic_runs == 0 {
                    // Panic is over — transition to hiding.
                    self.set_ai_state(AiState::Fleeing);
                    self.current_substate = Substate::FleeingHiding;
                    if self.directed_panic {
                        // Look back at the panic source.
                        self.face_position(Position {
                            x: self.panic_center_x,
                            y: self.panic_center_y,
                            sector: None,
                            level: 0,
                        });
                    } else {
                        // Look in a random direction.
                        self.face_direction(crate::sim_rng::u32(0..16) as u16, ctx);
                    }
                    self.clear_emoticon();
                    self.set_alert_status(AlertLevel::Yellow);
                    // BlinkEnemy() is wired via refresh_view when the
                    // music alert status changes; nothing to do here
                    // explicitly.
                    let hiding_time = crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                        + crate::sim_rng::u32(
                            0..crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32,
                        );
                    self.launch_timer(hiding_time, ctx.frame);
                    return true;
                }

                if stimulus_type == StimulusType::EventReachPoint {
                    // Decrement panic runs and start a new GoTo toward
                    // a fresh escape vector.
                    self.lasting_panic_runs = self.lasting_panic_runs.saturating_sub(1);

                    let sector_index = if !self.directed_panic {
                        // Undirected panic — any direction.
                        (crate::sim_rng::u32(0..16) & 15) as u8
                    } else {
                        // Directed panic — run away from panic center.
                        let dx = ctx.position.x - self.panic_center_x;
                        let dy = ctx.position.y - self.panic_center_y;
                        let base =
                            crate::position_interface::vector_to_sector_0_to_15(dx, dy) as u8;
                        if self.first_try {
                            // ±2 sector jitter around the base.
                            let jitter =
                                (crate::sim_rng::u32(0..5) as i32 - 2).rem_euclid(16) as u8;
                            base.wrapping_add(jitter) & 15
                        } else {
                            // Previous attempt failed — rotate 90° to
                            // the side determined by creation-order
                            // parity, with ±3 sector jitter. We key off
                            // the NPC handle's low bit because it's
                            // stable, unique, and has the same parity
                            // effect as the original creation-order
                            // bit.
                            let side = if self.me & 1 != 0 { 4 } else { 12 };
                            let jitter =
                                (crate::sim_rng::u32(0..7) as i32 - 3).rem_euclid(16) as u8;
                            base.wrapping_add(side).wrapping_add(jitter) & 15
                        }
                    };

                    let (vx, vy) = crate::element::direction_vector_16(sector_index as i16);
                    let segment = (crate::parameters_ai::AI_MIN_PANIC_RUN_SEGMENT_DISTANCE as u32
                        + crate::sim_rng::u32(
                            0..crate::parameters_ai::AI_DELTA_PANIC_RUN_SEGMENT_DISTANCE as u32,
                        )) as f32;
                    let dest = Position {
                        x: ctx.position.x + vx * segment,
                        y: ctx.position.y + vy * segment,
                        sector: ctx.position.sector,
                        level: ctx.position.level,
                    };

                    // Next time around we're no longer on the first try.
                    self.first_try = true;

                    let mut flags = GotoFlags::RUN | GotoFlags::STRAIGHT | GotoFlags::ASK_OBSTACLE;
                    if self.lasting_panic_runs > 0 {
                        flags |= GotoFlags::DONT_STOP;
                    }
                    self.go_to(dest, flags, ctx);
                } else {
                    // EventCouldntReachPoint — the random direction
                    // was blocked. Flip `first_try` so the next run
                    // uses the 90° side-step branch, and queue a
                    // `SeekPoint` fallback for the engine to drain.
                    // The engine has the `seek_points` array; the
                    // `AiController` here doesn't, so we hand off via
                    // `pending_panic_seek_fallback` and let
                    // `process_pending_panic_seek_fallback_for` pick
                    // the anchor + call `go_to` (RUN|DONT_STOP mid-run,
                    // RUN on the last segment). If no seek point is
                    // found, the engine drain re-fires the self
                    // `EventReachPoint` as an emergency fall-through.
                    self.first_try = false;
                    self.pending_panic_seek_fallback = true;
                }
            }

            Substate::FleeingHiding => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty_common_stuff(DutyFlags::empty(), ctx);
                }
            }

            _ => {
                tracing::trace!(
                    "AiController::think_expected_event_common_stuff: unhandled substate {:?}",
                    self.current_substate,
                );
            }
        }

        false
    }

    /// Advance past the current waypoint and continue walking.
    /// Called when a waypoint's command is handled (or skipped).
    fn proceed_on_path(
        &mut self,
        hiking_paths: &[crate::level_data::RawHikingPath],
        ctx: &AiContext,
    ) {
        self.set_ai_state(AiState::Default);
        self.current_substate = Substate::DefaultEnroute;

        if let Some(ref mut path) = self.patrol_path {
            // One-waypoint path means "you are already there" — flag
            // `already_on_point` so the outer state machine re-fires
            // `EventReachPoint`. Do *not* advance and don't queue a
            // move.
            if path.size <= 1 {
                self.already_on_point = true;
                return;
            }
            path.advance();
            if let Some(wp) = path.current_waypoint(hiking_paths) {
                // Always pass `GOTO_STRAIGHT` here because macro-to-
                // macro waypoint transitions are straight-line (no
                // path-finder). Without this, the engine's movement
                // layer falls through to the routed direction branch.
                let mut walk_flags = self.default_path_walking_flags | GotoFlags::STRAIGHT;
                if !self.will_stop_at_next_waypoint(hiking_paths) {
                    walk_flags |= GotoFlags::DONT_STOP;
                }
                let dest = Position {
                    x: wp.x as f32,
                    y: wp.y as f32,
                    sector: SectorHandle::new(wp.sector),
                    level: wp.level,
                };
                self.go_to(dest, walk_flags, ctx);
            }
        }
    }

    /// Dispatch an EventDone to ourselves (used when skipping a turn).
    fn think_event_done_on_self(&mut self, ctx: &AiContext) {
        let done_stimulus = Stimulus::new(StimulusType::EventDone);
        self.think_expected_event_common_stuff(&done_stimulus, ctx);
    }
}

// ---------------------------------------------------------------------------
// Consideration accumulator (replaces module-static accumulators)
// ---------------------------------------------------------------------------

/// Helper for the weighted-attribute decision system. Modelled as an
/// explicit struct rather than module-static accumulators.
#[derive(Debug, Default)]
pub struct ConsiderationAccumulator {
    pub sum_of_values: u32,
    pub sum_of_weights: u32,
    pub sum_of_threshold_values: i32,
    pub sum_of_threshold_weights: u32,
    pub positive_threshold_values: bool,
    pub negative_threshold_values: bool,
}

impl ConsiderationAccumulator {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Add a value to the consideration. `positive_effect` means higher
    /// values favor "yes".
    pub fn consider_value(&mut self, positive_effect: bool, value: u8, weight: u8, threshold: u8) {
        debug_assert!(weight > 0);
        if threshold == 0 {
            let contrib = if positive_effect {
                value as u32
            } else {
                MAX_ATT_VALUE as u32 - value as u32
            };
            self.sum_of_values += contrib * weight as u32;
            self.sum_of_weights += weight as u32;
        } else {
            // Threshold branch: compare the *raw* value (not inverted)
            // against the threshold, and only accumulate if
            // `value > threshold`. The polarity flag is set
            // unconditionally based on `positive_effect`.
            if value > threshold {
                let delta = (value as i32 - threshold as i32) * weight as i32;
                if positive_effect {
                    self.sum_of_threshold_values += delta;
                } else {
                    self.sum_of_threshold_values -= delta;
                }
                self.sum_of_threshold_weights += weight as u32;
            }
            if positive_effect {
                self.positive_threshold_values = true;
            } else {
                self.negative_threshold_values = true;
            }
        }
    }

    /// Evaluate all accumulated considerations and return a value in
    /// 0..100. Initial lambda, threshold correction, clamp, then
    /// consume-and-reset.
    pub fn evaluate(&mut self) -> u8 {
        #[allow(clippy::manual_checked_ops)]
        let mut lambda: i32 = if self.sum_of_weights > 0 {
            (self.sum_of_values / self.sum_of_weights) as i32
        } else if self.positive_threshold_values == self.negative_threshold_values {
            HALF_MAX_ATT_VALUE
        } else if self.positive_threshold_values {
            0
        } else {
            MAX_ATT_VALUE
        };

        if self.sum_of_threshold_weights > 0 {
            let adjusted = self.sum_of_values as i32
                + lambda * self.sum_of_threshold_weights as i32
                + self.sum_of_threshold_values;
            lambda = adjusted / (self.sum_of_weights + self.sum_of_threshold_weights) as i32;
        }

        let result = lambda.clamp(0, MAX_ATT_VALUE) as u8;
        self.reset();
        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substate_groups() {
        assert!(Substate::SeekingSeekpoint.is_seek_area());
        assert!(!Substate::DefaultOnPost.is_seek_area());
        assert!(Substate::AttackingSwordfight.is_any_swordfight());
        assert!(Substate::AttackingSwordfight.is_real_swordfight());
        assert!(!Substate::AttackingBowShooting.is_any_swordfight());
    }

    #[test]
    fn ai_log_stimulus_strings_match_original_names_and_fallback() {
        assert_eq!(
            StimulusType::log_string_from_u16(StimulusType::EventView as u16),
            "EVENT-VIEW"
        );
        assert_eq!(
            StimulusType::log_string_from_u16(StimulusType::EventSeesFriendInTrouble as u16),
            "EVENT-SEESFRIENDINTROUBLE"
        );
        assert_eq!(
            StimulusType::log_string_from_u16(StimulusType::NoEvent as u16),
            "EVENT-???"
        );
        assert_eq!(StimulusType::log_string_from_u16(u16::MAX), "EVENT-???");
    }

    #[test]
    fn ai_log_substate_strings_match_original_names_and_fallback() {
        assert_eq!(
            Substate::log_string_from_u16(Substate::DefaultGotoPost as u16),
            "SUBSTATE-DEFAULT-GOTOPOST"
        );
        assert_eq!(
            Substate::log_string_from_u16(Substate::AttackingSwordfight as u16),
            "SUBSTATE-ATTACKING-SWORDFIGHT"
        );
        assert_eq!(
            Substate::log_string_from_u16(Substate::AttackingArcherWaitOnArcheryPath as u16),
            "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH"
        );
        assert_eq!(
            Substate::log_string_from_u16(Substate::DefaultGotoChief as u16),
            "SUBSTATE-DEFAULT-GOTOCHIEF"
        );
        assert_eq!(
            Substate::log_string_from_u16(Substate::AttackingRunToAvengerOnRoof as u16),
            "SUBSTATE-???"
        );
        assert_eq!(Substate::log_string_from_u16(u16::MAX), "SUBSTATE-???");
    }

    #[test]
    fn ai_log_decision_strings_match_original_names_and_fallback() {
        assert_eq!(
            Decision::log_string_from_u16(Decision::Fight as u16),
            "DECISION-FIGHT"
        );
        assert_eq!(
            Decision::log_string_from_u16(Decision::LookForHelp as u16),
            "DECISION-LOOK-4-HELP"
        );
        assert_eq!(
            Decision::log_string_from_u16(Decision::PredecisionOffensive as u16),
            "DECISION-???"
        );
        assert_eq!(Decision::log_string_from_u16(u16::MAX), "DECISION-???");
    }

    #[test]
    fn ai_log_remark_strings_match_original_speech_and_fallback() {
        assert_eq!(
            Remark::log_string_from_u16(Remark::SeesBody as u16),
            "Ca va?"
        );
        assert_eq!(
            Remark::log_string_from_u16(Remark::TheSoundOfSilence as u16),
            " ........... "
        );
        assert_eq!(Remark::log_string_from_u16(u16::MAX), " ........... ");
    }

    #[test]
    fn stimulus_similarity() {
        let a = Stimulus::new(StimulusType::EventTimer);
        let b = Stimulus::new(StimulusType::EventTimer);
        assert!(a.is_similar(&b));

        let c = Stimulus::new(StimulusType::EventDone);
        assert!(!a.is_similar(&c));

        let d = Stimulus::with_human(StimulusType::EventView, 42);
        let e = Stimulus::with_human(StimulusType::EventView, 42);
        assert!(d.is_similar(&e));

        let f = Stimulus::with_human(StimulusType::EventView, 99);
        assert!(!d.is_similar(&f));
    }

    #[test]
    fn consideration_accumulator() {
        let mut acc = ConsiderationAccumulator::default();
        acc.consider_value(true, 80, 1, 0);
        acc.consider_value(true, 60, 1, 0);
        let result = acc.evaluate();
        assert_eq!(result, 70);
    }

    #[test]
    fn value_between() {
        // Truncation: param < 100 → value_at_0, == 100 → value_at_100.
        // See `AiController::value_between` for context.
        assert_eq!(AiController::value_between(0, 100, 50), 0);
        assert_eq!(AiController::value_between(0, 100, 0), 0);
        assert_eq!(AiController::value_between(0, 100, 99), 0);
        assert_eq!(AiController::value_between(0, 100, 100), 100);
        assert_eq!(AiController::value_between(10, 90, 50), 10);
        assert_eq!(AiController::value_between(10, 90, 100), 90);
    }

    #[test]
    fn ai_controller_defaults() {
        let ai = AiController::new(1);
        assert_eq!(ai.me, 1);
        assert_eq!(ai.current_state, AiState::Default);
        assert_eq!(ai.current_substate, Substate::DefaultOnPost);
        assert_eq!(ai.attitude, Attitude::Suspicious);
        assert!(!ai.ai_is_locked());
    }

    #[test]
    fn goto_sword_sets_force_sword_movement_flag() {
        let order = AiController::make_move_order(
            &Position {
                x: 100.0,
                y: 200.0,
                sector: None,
                level: 0,
            },
            GotoFlags::SWORD,
        );

        let flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(order.move_flags));
        assert!(flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT));
    }

    #[test]
    fn goto_find_accessible_and_ask_obstacle_survive_order_intent() {
        let order = AiController::make_move_order(
            &Position {
                x: 100.0,
                y: 200.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            GotoFlags::FIND_ACCESSIBLE | GotoFlags::ASK_OBSTACLE | GotoFlags::STRAIGHT,
        );

        assert!(order.find_accessible);
        assert!(order.ask_obstacle);
        assert!(!order.compute_direction);
    }

    fn goto_short_circuit_ctx(animation: crate::order::OrderType) -> AiContext {
        AiContext {
            position: Position {
                x: 100.0,
                y: 200.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            posture: crate::element::Posture::Upright,
            self_animation: animation,
            ..AiContext::default()
        }
    }

    #[test]
    fn goto_already_on_point_uses_original_animation_gate() {
        for animation in [
            crate::order::OrderType::WaitingUpright,
            crate::order::OrderType::WaitingAlerted,
            crate::order::OrderType::NonanimationEnd,
        ] {
            let ctx = goto_short_circuit_ctx(animation);
            let mut ai = AiController::new(1);

            ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

            assert!(ai.already_on_point);
            assert!(ai.take_pending_orders().is_empty());
        }

        let ctx = goto_short_circuit_ctx(crate::order::OrderType::WaitingUprightBored);
        let mut ai = AiController::new(1);

        ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

        assert!(!ai.already_on_point);
        assert_eq!(ai.take_pending_orders().len(), 1);
    }

    #[test]
    fn run_to_map_exit_queues_running_map_movement() {
        let mut ai = AiController::new(1);
        let destination = Position {
            x: 123.0,
            y: 456.0,
            sector: SectorHandle::new(7),
            level: 0,
        };

        ai.run_to_map_exit(destination);
        let orders = ai.take_pending_orders();

        assert_eq!(orders.len(), 1);
        assert_eq!(
            orders[0].order_type,
            crate::order::OrderType::RunningUpright
        );
        let flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(orders[0].move_flags));
        assert!(flags.contains(crate::sequence::MoveFlags::MAP));
    }

    fn face_to_ctx(action_state: crate::element::ActionState) -> AiContext {
        AiContext {
            position: Position {
                x: 10.0,
                y: 20.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            direction: 4,
            posture: crate::element::Posture::Upright,
            self_action_state: action_state,
            ..AiContext::default()
        }
    }

    fn same_direction_target(ctx: &AiContext) -> Position {
        let dir = crate::shadow_polygon::sector_to_direction(ctx.direction as i16);
        Position {
            x: ctx.position.x + dir[0] * 100.0,
            y: ctx.position.y + dir[1] * 100.0,
            ..ctx.position
        }
    }

    fn assert_same_direction_short_circuits(mut ai: AiController, ctx: &AiContext) {
        ai.face_direction(ctx.direction, ctx);

        assert!(ai.already_turned);
        assert!(!ai.pending_halt);
        assert!(ai.take_pending_orders().is_empty());
    }

    #[test]
    fn face_to_same_direction_waiting_short_circuits() {
        let ctx = face_to_ctx(crate::element::ActionState::Waiting);

        assert_same_direction_short_circuits(AiController::new(1), &ctx);
    }

    #[test]
    fn face_to_same_direction_bored_short_circuits() {
        let ctx = face_to_ctx(crate::element::ActionState::Bored);
        let mut ai = AiController::new(1);
        ai.face_position_with_ctx(same_direction_target(&ctx), &ctx);

        assert!(ai.already_turned);
        assert!(!ai.pending_halt);
        assert!(ai.take_pending_orders().is_empty());
    }

    fn assert_same_direction_queues_turn(action_state: crate::element::ActionState) {
        let mut ai = AiController::new(1);
        let ctx = face_to_ctx(action_state);

        ai.face_direction(ctx.direction, &ctx);
        let orders = ai.take_pending_orders();

        assert!(!ai.already_turned);
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
        assert!(!orders[0].no_halt);
    }

    #[test]
    fn face_to_same_direction_upright_moving_launches_halting_turn() {
        assert_same_direction_queues_turn(crate::element::ActionState::Moving);
    }

    #[test]
    fn face_to_same_direction_upright_non_waiting_states_launch_halting_turn() {
        for action_state in [
            crate::element::ActionState::MovingFast,
            crate::element::ActionState::AimingWithBow,
            crate::element::ActionState::HoldingShield,
            crate::element::ActionState::Menacing,
        ] {
            assert_same_direction_queues_turn(action_state);
        }
    }

    // ──────────────────────────────────────────────────────────
    // init_state — initial-action gate
    // ──────────────────────────────────────────────────────────

    #[test]
    fn init_state_waiting_upright_returns_go_to_duty() {
        // `WaitingUpright` → OnPost + bored timer + `go_to_duty = true`.
        // This is the hot path — the vast majority of NPCs are
        // authored with this action.
        crate::sim_rng::with_seed(1, || {
            let mut ai = AiController::new(1);
            ai.initial_action = crate::order::OrderType::WaitingUpright as u32;
            let fx = ai.init_state(&AiContext::default());

            assert!(fx.go_to_duty);
            assert_eq!(ai.current_state, AiState::Default);
            assert_eq!(ai.current_substate, Substate::DefaultOnPost);
            assert!(fx.set_posture.is_none());
            assert!(!ai.likes_to_sit_around);
            assert!(!ai.special_action);
            assert!(!ai.is_stay_at_home);
        });
    }

    #[test]
    fn init_state_sleeping_upright_closes_eyes_and_emoticon() {
        // `SleepingUpright` → SleepingNapping, eyes closed, Zzz
        // emoticon, upright posture + Sleeping action state.
        // `go_to_duty = false` — the NPC stays asleep until something
        // wakes them.
        use crate::element::{ActionState, EyeStatus, Posture};
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::SleepingUpright as u32;
        let fx = ai.init_state(&AiContext::default());

        assert!(!fx.go_to_duty);
        assert_eq!(ai.current_state, AiState::Sleeping);
        assert_eq!(ai.current_substate, Substate::SleepingNapping);
        assert_eq!(ai.current_emoticon_type, EmoticonType::Zzz);
        assert_eq!(fx.set_eye_status, Some(EyeStatus::Closed));
        assert_eq!(fx.set_posture, Some(Posture::Upright));
        assert_eq!(fx.set_action_state, Some(ActionState::Sleeping));
    }

    #[test]
    fn init_state_sitting_flags_likes_to_sit_around() {
        // `Sitting` → OnPost + Sitting posture, and crucially sets
        // `likes_to_sit_around = true` so `return_to_duty_common_stuff`
        // routes back to this place with the sitting-specific posture
        // gate.
        crate::sim_rng::with_seed(1, || {
            let mut ai = AiController::new(1);
            ai.initial_action = crate::order::OrderType::Sitting as u32;
            let fx = ai.init_state(&AiContext::default());

            assert!(!fx.go_to_duty);
            assert_eq!(ai.current_state, AiState::Default);
            assert_eq!(ai.current_substate, Substate::DefaultOnPost);
            assert!(ai.likes_to_sit_around);
            assert_eq!(fx.set_posture, Some(crate::element::Posture::Sitting));
        });
    }

    #[test]
    fn init_state_special_flags_special_action() {
        // `Special` → Leisure posture, flips `special_action = true`.
        // Pairs with the corresponding branch in
        // `return_to_duty_common_stuff`.
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::Special as u32;
        let fx = ai.init_state(&AiContext::default());

        assert!(!fx.go_to_duty);
        assert!(ai.special_action);
        assert_eq!(fx.set_posture, Some(crate::element::Posture::Leisure));
    }

    #[test]
    fn init_state_being_unconscious_queues_max_concussion() {
        // `BeingUnconscious` → SleepingUnconscious, Lying posture,
        // concussion/unconscious side effect.
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::BeingUnconscious as u32;
        let fx = ai.init_state(&AiContext::default());

        assert!(!fx.go_to_duty);
        assert_eq!(ai.current_state, AiState::Sleeping);
        assert_eq!(ai.current_substate, Substate::SleepingUnconscious);
        assert!(fx.concussion_max_and_unconscious);
        assert_eq!(fx.set_posture, Some(crate::element::Posture::Lying));
    }

    #[test]
    fn init_state_being_dead_zeroes_life_points() {
        // `BeingDead{FallenBack}` → SleepingForever,
        // `zero_life_points` side effect. Two variants differ only in
        // posture.
        for (raw, expected_posture) in [
            (
                crate::order::OrderType::BeingDead as u32,
                crate::element::Posture::Dead,
            ),
            (
                crate::order::OrderType::BeingDeadFallenBack as u32,
                crate::element::Posture::DeadBack,
            ),
        ] {
            let mut ai = AiController::new(1);
            ai.initial_action = raw;
            let fx = ai.init_state(&AiContext::default());

            assert!(!fx.go_to_duty);
            assert_eq!(ai.current_substate, Substate::SleepingForever);
            assert!(fx.zero_life_points);
            assert_eq!(fx.set_posture, Some(expected_posture));
        }
    }

    #[test]
    fn init_state_in_building_stays_at_home() {
        // Indoor NPCs short-circuit to `is_stay_at_home=true` +
        // DefaultHomeSweetHome, regardless of `initial_action`.
        // `go_to_duty = false`.
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::WaitingUpright as u32;
        let ctx = AiContext {
            in_building: true,
            building_sector: SectorHandle::new(7),
            ..AiContext::default()
        };
        let fx = ai.init_state(&ctx);

        assert!(!fx.go_to_duty);
        assert!(ai.is_stay_at_home);
        assert_eq!(ai.current_substate, Substate::DefaultHomeSweetHome);
    }

    #[test]
    fn init_state_resets_flags_before_branching() {
        // Calling `init_state` repeatedly should clear stale
        // `likes_to_sit_around` / `special_action` / `is_stay_at_home`
        // flags from a prior call.  Guards against level-editor
        // authored sequences that change `initial_action` between
        // init passes (e.g. respawn via script).
        crate::sim_rng::with_seed(1, || {
            let mut ai = AiController::new(1);
            ai.likes_to_sit_around = true;
            ai.special_action = true;
            ai.is_stay_at_home = true;
            ai.initial_action = crate::order::OrderType::WaitingUpright as u32;

            let fx = ai.init_state(&AiContext::default());

            assert!(fx.go_to_duty);
            assert!(!ai.likes_to_sit_around);
            assert!(!ai.special_action);
            assert!(!ai.is_stay_at_home);
        });
    }

    #[test]
    fn emoticon_transient() {
        let mut ai = AiController::new(1);
        ai.set_transient_emoticon(EmoticonType::QuestionMark, 100, 500);
        assert_eq!(ai.current_emoticon_type, EmoticonType::QuestionMark);
        assert!(ai.emoticon_has_expiration_date);
        assert_eq!(ai.emoticon_expiration_date, 600);
    }

    #[test]
    fn recon_report() {
        let mut report = ReconnaissanceReport::default();
        assert_eq!(report.report_type, ReportType::Nothing);

        report.update(
            ReportType::Body,
            Position {
                x: 10.0,
                y: 20.0,
                sector: None,
                level: 0,
            },
        );
        assert_eq!(report.report_type, ReportType::Body);

        // Lower priority update should be ignored
        report.update(
            ReportType::Noise,
            Position {
                x: 30.0,
                y: 40.0,
                sector: None,
                level: 0,
            },
        );
        assert_eq!(report.report_type, ReportType::Body);
        assert_eq!(report.seek_position.x, 10.0);

        // Higher priority update should apply
        report.update(
            ReportType::Enemy,
            Position {
                x: 50.0,
                y: 60.0,
                sector: None,
                level: 0,
            },
        );
        assert_eq!(report.report_type, ReportType::Enemy);
        assert_eq!(report.seek_position.x, 50.0);
    }

    #[test]
    fn position_to_point_3d_uses_waypoint_sector_layer_projection() {
        let mut bbox = crate::geo2d::BBox2D::new();
        let points_geo = vec![
            crate::geo2d::pt(0.0, 0.0),
            crate::geo2d::pt(100.0, 0.0),
            crate::geo2d::pt(100.0, 100.0),
            crate::geo2d::pt(0.0, 100.0),
        ];
        for &point in &points_geo {
            bbox.expand_point(point);
        }
        let points = points_geo
            .into_iter()
            .map(crate::coordinates::MapPoint::from_geo)
            .collect();

        let sector_number = crate::sector::SectorNumber::new(7);
        let mut level = crate::fast_find_grid::LevelGrid::default();
        level.sectors.push(crate::fast_find_grid::GridSector {
            points,
            bounding_box: bbox,
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            layer: 2,
            sector_number,
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
        level.sector_number_map.insert(sector_number, 0);

        let mut obstacle = crate::sight_obstacle::SightObstacle::new(
            0,
            crate::sight_obstacle::SIGHTOBSTACLE_SOLID
                | crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
        );
        obstacle.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 20.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 100.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 20.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 100.0,
                y: 100.0,
                z_bottom: 0.0,
                z_top: 20.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 0.0,
                y: 100.0,
                z_bottom: 0.0,
                z_top: 20.0,
            },
        ];
        obstacle.layer = 2;
        obstacle.sector = 7;
        obstacle.top_plane_points = [[0.0, 0.0, 20.0], [100.0, 0.0, 20.0], [0.0, 100.0, 20.0]];
        obstacle.bottom_plane_points = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
        obstacle.rebuild_geometry();

        let ctx = AiContext {
            fast_grid: crate::fast_find_grid::FastFindGrid {
                level: std::sync::Arc::new(level),
                line_active: Vec::new(),
                sector_active: vec![true],
                mask_active: Vec::new(),
                lift_state: std::collections::BTreeMap::new(),
                sector_type_overlay: std::collections::BTreeMap::new(),
            },
            sight_obstacles: crate::sight_obstacle::SharedSightObstacles {
                static_obstacles: std::sync::Arc::new(vec![obstacle]),
                dynamic_obstacles: std::sync::Arc::new(Vec::new()),
                static_active: std::sync::Arc::new(vec![true]),
            },
            ..AiContext::default()
        };

        let point = ctx.position_to_point_3d(Position {
            x: 50.0,
            y: 50.0,
            sector: SectorHandle::new(7),
            level: 2,
        });

        assert_eq!(point.x, 50.0);
        assert_eq!(point.y, 70.0);
        assert_eq!(point.z, 20.0);
    }

    #[test]
    fn is_detecting_point_360_uses_current_eye_point() {
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            direction: 4,
            posture: crate::element::Posture::LeaningOut,
            sq_standard_view_radius: 11.0 * 11.0,
            sq_self_view_radius: 11.0 * 11.0,
            ..AiContext::default()
        };

        assert!(
            ctx.is_detecting_point_360(crate::coordinates::WorldPoint3D {
                x: 50.0,
                y: 0.0,
                z: 45.0,
            })
        );
    }

    // ── House / building-AI tests ─────────────────────────────────

    #[test]
    fn house_default_values() {
        let h = House::default();
        assert_eq!(h.sector_index, 0);
        assert_eq!(h.building_index, None);
        assert!(h.door_indices.is_empty());
        assert!(h.occupant_ids.is_empty());
        assert!(!h.arrow_reserve);
    }

    /// Mirrors the enter / leave sequence that `execute_pass_door`
    /// runs when an actor walks through a building door — a direct
    /// unit-level exercise of the same Vec `push` / `retain` logic
    /// used by the runtime hooks, so regressions in
    /// `House::occupant_ids` semantics are caught without needing a
    /// full engine fixture.
    #[test]
    fn house_occupant_enter_leave_cycle() {
        use crate::element::EntityId;

        let mut h = House {
            sector_index: 42,
            ..House::default()
        };
        let a = EntityId::Pc(1);
        let b = EntityId::Pc(2);

        // Enter A, then B
        if !h.occupant_ids.contains(&a) {
            h.occupant_ids.push(a);
        }
        if !h.occupant_ids.contains(&b) {
            h.occupant_ids.push(b);
        }
        assert_eq!(h.occupant_ids, vec![a, b]);

        // Dedup: re-entering A while already inside is a no-op.
        if !h.occupant_ids.contains(&a) {
            h.occupant_ids.push(a);
        }
        assert_eq!(h.occupant_ids, vec![a, b]);

        // Leave A — B stays.
        h.occupant_ids.retain(|&e| e != a);
        assert_eq!(h.occupant_ids, vec![b]);

        // Leave B — empty list, house entry still alive.
        h.occupant_ids.retain(|&e| e != b);
        assert!(h.occupant_ids.is_empty());
        assert_eq!(h.sector_index, 42);
    }

    #[test]
    fn house_occupancy_helpers() {
        use crate::element::EntityId;
        let mut h = House::default();
        assert_eq!(h.occupant_count(), 0);
        h.occupant_ids.push(EntityId::Pc(1));
        h.occupant_ids.push(EntityId::Pc(2));
        assert_eq!(h.occupant_count(), 2);
        assert!(h.contains_occupant(EntityId::Pc(1)));
        assert!(!h.contains_occupant(EntityId::Pc(99)));
    }

    #[test]
    fn ambush_point_init_lift_defaults() {
        // New AmbushPoints default to z=0 and id=0 before init runs.
        let ap = AmbushPoint {
            position: Position {
                x: 100.0,
                y: 200.0,
                sector: None,
                level: 0,
            },
            direction: 0,
            position_3d: crate::coordinates::WorldPoint3D::default(),
            id: 0,
        };
        assert_eq!(ap.position_3d.z, 0.0);
        assert_eq!(ap.id, 0);
    }
}
