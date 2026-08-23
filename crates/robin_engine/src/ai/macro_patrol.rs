use super::*;

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

/// Exact serialized `RHPath::SerializeStatus` state retained while no hiking
/// path is attached. `RHPath::Init(-1)` only clears the path pointer/index; it
/// deliberately preserves these cursor, direction, and history values.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct DetachedPatrolPathStatus {
    pub hiking_path_index: Option<PathId>,
    pub current_waypoint_index: u8,
    pub last_waypoint_index: u8,
    pub forward: bool,
    pub history: Vec<PathHistoryEntry>,
}

impl Default for DetachedPatrolPathStatus {
    fn default() -> Self {
        Self {
            hiking_path_index: None,
            current_waypoint_index: 0,
            last_waypoint_index: 0,
            forward: true,
            history: Vec::new(),
        }
    }
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
        ctx: &crate::ai::AiContext,
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
                    sector: ctx.hiking_waypoint_sector(
                        usize::from(self.hiking_path_index),
                        i,
                        wp.sector,
                    ),
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
        chief_move_box: &crate::coordinates::MoveBox,
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
            let on_path = MapPoint::new(entry.position.x, entry.position.y);
            let mut chosen = sidewards;
            if let Some(grid) = fast_grid {
                const FALLBACK_SCALES: &[f32] = &[1.0, 0.6, 0.3, 0.0];
                for &scale in FALLBACK_SCALES {
                    let candidate = MapPoint::new(
                        on_path.x + sidewards[0] * scale,
                        on_path.y + sidewards[1] * scale,
                    );
                    if scale == 0.0
                        || grid.is_straight_movement_authorized(
                            on_path,
                            candidate,
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ForecastedDestination {
    pub position: Position,
    pub direction: u16,
}

/// RNG-free destination forecast prepared from live actor/door state.
/// Building exits remain alternatives until the exact AI consumer resolves
/// the forecast, preserving Original draw ownership.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PreparedForecastDestination {
    fallback: ForecastedDestination,
    building_gates: Vec<ForecastedDestination>,
    entry_gate: Option<usize>,
    /// Whether `ForecastDestinationForIA` assigns its `UWORD& uwDirection`
    /// output on this deterministic branch. A one-gate building enters the
    /// building branch, but the `uwNumberOfGates > 1` body does not run, so
    /// Original leaves the caller's previous value untouched.
    direction_written: bool,
}

impl PreparedForecastDestination {
    pub fn fixed(position: Position, direction: u16) -> Self {
        Self {
            fallback: ForecastedDestination {
                position,
                direction,
            },
            building_gates: Vec::new(),
            entry_gate: None,
            direction_written: true,
        }
    }

    pub fn resolve(&self, sim: &crate::sim_rng::SimulationContext) -> ForecastedDestination {
        if self.building_gates.is_empty() {
            return self.fallback;
        }
        assert!(
            self.building_gates.len() > 1
                && self
                    .entry_gate
                    .is_none_or(|entry_gate| entry_gate < self.building_gates.len()),
            "prepared building forecast has an invalid entry gate"
        );
        loop {
            let selected = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::BuildingExitGate,
                ..self.building_gates.len(),
            );
            // When GetDoor() is already NULL, Original compares the selected
            // gate against NULL and accepts every real gate on the first draw.
            if self.entry_gate != Some(selected) {
                return self.building_gates[selected];
            }
        }
    }

    /// Resolve while preserving the caller-owned direction when Original's
    /// output-reference branch does not assign it.
    pub fn resolve_retaining_direction(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        retained_direction: u16,
    ) -> ForecastedDestination {
        let mut resolved = self.resolve(sim);
        if !self.direction_written {
            resolved.direction = retained_direction;
        }
        resolved
    }
}

#[cfg(test)]
mod prepared_forecast_tests {
    use super::{ForecastedDestination, Position, PreparedForecastDestination};
    use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

    #[test]
    fn building_exit_rejects_entry_against_the_full_ordered_gate_list() {
        let prepared = PreparedForecastDestination {
            fallback: ForecastedDestination {
                position: Position::default(),
                direction: 0,
            },
            building_gates: vec![
                ForecastedDestination {
                    position: Position {
                        x: 10.0,
                        ..Position::default()
                    },
                    direction: 1,
                },
                ForecastedDestination {
                    position: Position {
                        x: 20.0,
                        ..Position::default()
                    },
                    direction: 2,
                },
                ForecastedDestination {
                    position: Position {
                        x: 30.0,
                        ..Position::default()
                    },
                    direction: 3,
                },
            ],
            entry_gate: Some(1),
            direction_written: true,
        };

        let seed = (0..10_000)
            .find(|seed| {
                let sim = SimulationContext::with_seed(*seed);
                crate::sim_rng::usize(&sim, RngSite::BuildingExitGate, ..3) == 1
                    && crate::sim_rng::usize(&sim, RngSite::BuildingExitGate, ..3) != 1
            })
            .expect("find a deterministic entry-then-exit draw sequence");
        let expected_sim = SimulationContext::with_seed(seed);
        assert_eq!(
            crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..3),
            1
        );
        let expected_exit = crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..3);

        let sim = SimulationContext::with_seed(seed);
        let (resolved, trace) = with_draw_trace(|| prepared.resolve(&sim));
        assert_eq!(
            resolved.position.x,
            prepared.building_gates[expected_exit].position.x
        );
        assert_eq!(
            resolved.direction, prepared.building_gates[expected_exit].direction,
            "rejection must retain the Original all-gates index mapping"
        );
        assert_eq!(
            trace,
            vec![RngSite::BuildingExitGate, RngSite::BuildingExitGate],
            "selecting the entry gate must consume another authoritative draw"
        );
    }

    #[test]
    fn building_exit_without_a_live_entry_gate_accepts_the_first_draw() {
        let prepared = PreparedForecastDestination {
            fallback: ForecastedDestination {
                position: Position::default(),
                direction: 0,
            },
            building_gates: vec![
                ForecastedDestination {
                    position: Position {
                        x: 10.0,
                        ..Position::default()
                    },
                    direction: 1,
                },
                ForecastedDestination {
                    position: Position {
                        x: 20.0,
                        ..Position::default()
                    },
                    direction: 2,
                },
            ],
            entry_gate: None,
            direction_written: true,
        };

        let sim = SimulationContext::with_seed(17);
        let expected_sim = SimulationContext::with_seed(17);
        let expected = crate::sim_rng::usize(&expected_sim, RngSite::BuildingExitGate, ..2);
        let (resolved, trace) = with_draw_trace(|| prepared.resolve(&sim));

        assert_eq!(
            resolved.position.x,
            prepared.building_gates[expected].position.x
        );
        assert_eq!(trace, vec![RngSite::BuildingExitGate]);
    }

    #[test]
    fn one_gate_building_keeps_the_callers_direction_output() {
        // RHelementactorhuman.cpp:13560-13588 enters the building branch,
        // but assigns uwDirection only inside `uwNumberOfGates > 1`.
        let prepared = PreparedForecastDestination {
            fallback: ForecastedDestination {
                position: Position {
                    x: 985.0,
                    y: 2597.0,
                    ..Position::default()
                },
                // The target currently faces 10, but this value must not
                // overwrite the caller-owned output on the one-gate branch.
                direction: 10,
            },
            building_gates: Vec::new(),
            entry_gate: None,
            direction_written: false,
        };

        let sim = SimulationContext::with_seed(1);
        let (resolved, trace) = with_draw_trace(|| prepared.resolve_retaining_direction(&sim, 0));

        assert_eq!(resolved.position.x, 985.0);
        assert_eq!(resolved.direction, 0);
        assert!(trace.is_empty());
    }
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
    /// Live `GetDoor()` pointer and its direction, when non-null.
    pub door_pass: Option<(crate::gate::DoorIndex, bool)>,
    /// Original's independent `mbPassingDoorDirectly` latch. It remains true
    /// after the first PassingDoor callback clears `GetDoor()` and still
    /// enables random exit forecasting from the actor's current building.
    pub passing_door_directly: bool,
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
    sim: &crate::sim_rng::SimulationContext,
    input: &ForecastInput,
    doors: &[crate::gate::Door],
    sectors: &[crate::fast_find_grid::GridSector],
    sector_map: &std::collections::HashMap<crate::sector::SectorNumber, usize>,
) -> ForecastedDestination {
    prepare_forecast_destination_for_ia(input, doors, sectors, sector_map).resolve(sim)
}

/// Opt-in provenance for `ForecastDestinationForIA` resolutions that run
/// through a lift sector. Process-local diagnostic state only: the value is
/// read once and never enters engine state, snapshots, hashes, or the
/// simulation RNG stream.
fn forecast_ia_debug_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PARITY_DEBUG_FORECAST_IA").is_some())
}

/// Prepare every deterministic branch of `ForecastDestinationForIA` without
/// consuming the authoritative RNG. Only [`PreparedForecastDestination::resolve`]
/// selects a building exit.
pub fn prepare_forecast_destination_for_ia(
    input: &ForecastInput,
    doors: &[crate::gate::Door],
    sectors: &[crate::fast_find_grid::GridSector],
    sector_map: &std::collections::HashMap<crate::sector::SectorNumber, usize>,
) -> PreparedForecastDestination {
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
                    MapPoint::new(input.position_map_x, input.position_map_y),
                    input.forecasted_movement_z > 0.0,
                    None,
                )
            }
        } else {
            // Not passing a door — use current position.
            (
                input.sector,
                input.layer,
                MapPoint::new(input.position_map_x, input.position_map_y),
                input.forecasted_movement_z > 0.0,
                None,
            )
        };
    let mut direction = input.direction;
    let mut building_gates = Vec::new();
    let mut entry_gate = None;
    let mut direction_written = true;

    // Look up the destination sector in the grid.
    let grid_sector = sector_map
        .get(&crate::sector::SectorNumber::new(sector as i16))
        .and_then(|&idx| sectors.get(idx));

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
        } else if gs.sector_type.is_building() && input.passing_door_directly {
            // Target entering a building (direct only) — predict exit
            // through a random gate. If GetDoor() is still live, reject that
            // entry gate; after PassDoor clears it, the NULL comparison in
            // Original accepts whichever real gate the first draw selects.
            // Direction uses `(PointOut - PointIn)`.
            for (door_index, door) in doors
                .iter()
                .enumerate()
                .filter(|(_, door)| door.sector_in == sector)
            {
                if let Some(current_door) = current_door_index {
                    if door_index as u32 == u32::from(current_door) {
                        entry_gate = Some(building_gates.len());
                    }
                }
                building_gates.push(ForecastedDestination {
                    position: Position {
                        x: door.point_out.x,
                        y: door.point_out.y,
                        sector: SectorHandle::new(u16::from(door.sector_out)),
                        level: door.layer_out,
                    },
                    direction: door_exit_direction_from_in(door),
                });
            }
            if building_gates.len() <= 1 {
                building_gates.clear();
                entry_gate = None;
                // Original has already selected the building `else if` here,
                // so it does not fall through to `uwDirection =
                // GetDirection()`. With no alternate exit, the output
                // reference retains its caller-owned value.
                direction_written = false;
            } else if let Some(current_door) = current_door_index {
                assert!(
                    entry_gate.is_some(),
                    "building sector {sector} has no entry door {} in its ordered gate list",
                    u32::from(current_door)
                );
            }
        }
        // else: position is fine, keep current direction.
    }

    if forecast_ia_debug_enabled() && grid_sector.is_some_and(|gs| gs.sector_type.is_lift()) {
        eprintln!(
            "FORECAST input={input:?} out=({}, {}, sector={sector}, layer={layer}) dir={direction} gates={} entry={entry_gate:?}",
            point.x,
            point.y,
            building_gates.len(),
        );
    }

    PreparedForecastDestination {
        fallback: ForecastedDestination {
            position: Position {
                x: point.x,
                y: point.y,
                sector: SectorHandle::new(sector),
                level: layer,
            },
            direction,
        },
        building_gates,
        entry_gate,
        direction_written,
    }
}

/// Find the exit door for a lift sector in the given direction.
///
/// `RHSectorLift` resolves `GetHighSector/Layer/EntryPoint/ExitDirection`
/// through `mpHighestDoor` and the low equivalents through `mpLowestDoor`
/// (`original-code/RHSector.h:315-327`). Those two doors are chosen at level
/// load by extreme `GetPointOut().mY` among the lift's own doors, *not* by
/// door type (`original-code/RHsector.cpp:1493-1521`) — the reference keeps
/// the `DOOR_LIFT_HIGH` assertion commented out at `RHsector.cpp:1524`
/// precisely because a lift's high endpoint often carries another type.
/// Selecting by type therefore misses the endpoint entirely on such lifts
/// and silently falls back to the target's raw position.
fn find_lift_exit_door(
    lift_sector: u16,
    moving_upwards: bool,
    doors: &[crate::gate::Door],
) -> Option<&crate::gate::Door> {
    let (low_index, high_index) = crate::gate::lift_endpoint_door_indices(
        doors,
        crate::sector::SectorNumber::new(lift_sector as i16),
    )?;
    let index = if moving_upwards {
        high_index
    } else {
        low_index
    };
    doors.get(index as usize)
}

/// Pick a random building exit gate that isn't the entry door.
///
/// Collects candidates and draws from the caller's explicit simulation stream.
/// Compute the exit direction from a door's geometry.
///
/// For lifts: `(GetPointOut() - GetPointMid()).GetSector0to15(ASPECT_RATIO)`.
/// For building gates: `(GetPointOut() - GetPointIn()).GetSector0to15(ASPECT_RATIO)`.
fn door_exit_direction_from_mid(door: &crate::gate::Door) -> u16 {
    let dx = door.point_out.x - door.point_mid.x;
    let dy = door.point_out.y - door.point_mid.y;
    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as u16
}

fn door_exit_direction_from_in(door: &crate::gate::Door) -> u16 {
    let dx = door.point_out.x - door.point_in.x;
    let dy = door.point_out.y - door.point_in.y;
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
