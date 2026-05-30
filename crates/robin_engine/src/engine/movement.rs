//! Movement ticking, pathfinding dispatch, and order processing.

use super::*;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::EntityId;
use crate::geo2d::{self, GeoPoint2D};
use crate::movement::ActiveMovement;
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

/// Per-entity lift translation snapshot consumed by `tick_entity_movement`'s
/// per-frame anim derivation.  Covers the lift cases of
/// `DetermineMovementAnimation`.
#[derive(Debug, Clone, Copy)]
enum LiftAnimContext {
    /// Upright posture in a lift sector.  Upwards and downwards animations
    /// are asserted equal for upright posture, so a single mapping covers
    /// both directions.
    Upright(crate::sector::LiftType),
    /// On-ladder / on-wall posture in a ladder or wall lift sector.  The
    /// per-frame upwards-vs-downwards pick comes from the dot product of
    /// the ladder vector (low point minus high point) with the actor's
    /// movement vector.  `ladder_dx` / `ladder_dy` is that ladder vector
    /// in map coordinates.
    OnClimb {
        lift_type: crate::sector::LiftType,
        lift_direction: i16,
        ladder_dx: f32,
        ladder_dy: f32,
    },
}

fn order_uses_distance_motion(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingAlerted
            | OrderType::RunningUpright
            | OrderType::WalkingWithSword
            | OrderType::RunningWithSword
            | OrderType::WalkingStairs
            | OrderType::WalkingStairsAlerted
            | OrderType::RunningStairs
            | OrderType::WalkingSword
            | OrderType::WalkingBackwardsSword
            | OrderType::StrafingRightSword
            | OrderType::StrafingLeftSword
            | OrderType::WalkingShield
            | OrderType::WalkingBackwardsShield
            | OrderType::StrafingRightShield
            | OrderType::StrafingLeftShield
            | OrderType::WalkingWithCorpse
            | OrderType::WalkingCarryingOnShoulders
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

fn sprite_motion_order_for_nonanimation(order: OrderType) -> OrderType {
    match order {
        // legacy implementation RHNONANIMATION_CLIMBING_*_FAST tokens are dispatch /
        // pathfinder speed tokens. RHElementActor handles them by
        // playing the normal climb animation row with RHMOTIONMETHOD_RUN.
        OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
        OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
        OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
        OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
        other => other,
    }
}

fn door_click_polygon_at(game_host: &crate::natives::GameHost, click: MapPoint) -> Option<u32> {
    game_host
        .doors
        .iter()
        .enumerate()
        .find(|(_, door)| door.is_door() && door.click_polygon_contains(click.x, click.y))
        .map(|(idx, _)| idx as u32)
}

fn is_sword_movement_nonanimation(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingWithSword | OrderType::RunningWithSword
    )
}

fn movement_execute_state_effect(
    order: OrderType,
    motion: MotionState,
) -> Option<(crate::element::Posture, crate::element::ActionState)> {
    use crate::element::{ActionState as AS, Posture as P};
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    match (order, motion) {
        (
            OT::TransitionWalkingUprightWaitingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::TransitionSpecialWaitingUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionWaitingUprightSpecial, MS::Done | MS::Terminated) => {
            Some((P::Leisure, AS::Waiting))
        }
        (OT::TransitionWaitingUprightBoredWaitingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Waiting))
        }
        (OT::TransitionWaitingUprightWaitingUprightBored, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Bored))
        }
        (
            OT::TransitionCrouchingUp
            | OT::TransitionSittingWaitingUpright
            | OT::TransitionLeaningOutWaitingAlerted
            | OT::LoweringShield,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionCrouchingDown, MS::Done | MS::Terminated) => {
            Some((P::Crouched, AS::Waiting))
        }
        (
            OT::TransitionWaitingUprightRunningUpright | OT::TransitionWalkingUprightRunningUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::MovingFast)),
        (OT::TransitionRunningUprightWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::WalkingUpright | OT::WalkingAlerted | OT::WalkingCrouched, MS::Start) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::RunningUpright, MS::Start) => Some((P::Upright, AS::MovingFast)),
        (OT::WalkingWithSword, MS::Start) => Some((P::Upright, AS::MovingSword)),
        (OT::RunningWithSword, MS::Start) => Some((P::Upright, AS::MovingFastSword)),
        _ => None,
    }
}

fn is_in_place_movement_transition(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::TransitionWaitingUprightSpecial
            | OrderType::TransitionSpecialWaitingUpright
            | OrderType::TransitionWaitingUprightBoredWaitingUpright
            | OrderType::TransitionWaitingUprightWaitingUprightBored
            | OrderType::TransitionCrouchingUp
            | OrderType::TransitionCrouchingDown
            | OrderType::TransitionSittingWaitingUpright
            | OrderType::TransitionLeaningOutWaitingAlerted
            | OrderType::StandingUp
            | OrderType::StandingUpSword
            | OrderType::StandingUpBow
            | OrderType::LoweringShield
    )
}

/// Result of [`EngineInner::advance_door_pass`].
///
/// Outcomes from draining the order list after a walk step terminates.
#[derive(Debug, Clone)]
pub(super) enum DoorPassAdvance {
    /// No active door pass existed when the state machine was asked to
    /// advance. This is a caller bug or a stale animation callback; it
    /// must not be treated as a completed pass.
    NoActive,
    /// A new `Walk` step is ready — the caller must push a walking
    /// order onto the actor's current sequence element to install the
    /// destination.  Movement tick resumes once the order is queued.
    Continue {
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        /// Walk-step tolerance copied from the source
        /// [`DoorPassStep::Walk`].  Populated for the ladder/wall
        /// translators and `0.0` for stairs/building/default.
        tolerance: f32,
    },
    /// A `Transition` step was popped — the caller must push the
    /// included [`crate::order::Order`] onto the actor's current
    /// sequence element and *not* clear `active_door_pass` or signal
    /// arrival.  Door-pass advancement resumes when the transition
    /// animation completes (via [`crate::order::OrderCompletion::ResumeDoorPass`]).
    Paused {
        transition_order: crate::order::Order,
    },
    /// No more steps remain; the door pass is complete and the caller
    /// should tear down path / active-movement state.
    Done {
        completed: Option<(crate::gate::DoorIndex, bool)>,
    },
}

// ─── Group-move formation helper ─────────────────────────────────────

/// Compute per-character destination points for a "mercenary"-style group
/// move around `click_point`.
///
/// The group's centroid is calculated, then each character's destination
/// is its current position translated so that the centroid lands on the
/// click point — preserving the relative formation of the group.
///
/// Returns a vector with the same length as `pc_positions`, each entry
/// being the destination for the PC at the matching index.  Returns an
/// empty vector if `pc_positions` is empty.
pub(crate) fn mercenary_formation_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    if pc_positions.is_empty() {
        return Vec::new();
    }

    let n = pc_positions.len() as f32;
    let cx = pc_positions.iter().map(|p| p.x).sum::<f32>() / n;
    let cy = pc_positions.iter().map(|p| p.y).sum::<f32>() / n;

    pc_positions
        .iter()
        .map(|p| MapPoint::new(p.x - cx + click_point.x, p.y - cy + click_point.y))
        .collect()
}

/// Shape of the goal passed to [`EngineInner::build_gate_movement_sequence`].
///
/// Unifies the three goal flavours (point, door, line) into a single
/// builder; the function switches on this enum to pick the right
/// trailing-step shape.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GoalShape {
    /// Point-goal. The actor walks to this map point after the last gate.
    Point(MapPoint),
    /// Entity-target seek goal.  The trailing MOVE keeps the target
    /// element, SEEK flag, and tolerance so arrival uses the same
    /// live target-distance predicate as a plain `Command::Seek`.
    Seek {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Door-goal.  The gate path's final element is the goal door
    /// itself.  `far_side_point` describes the point the actor lands
    /// at after passing through.  When the far-side sector is a
    /// building, a `CHANGE_POSITION` teleport is emitted.
    Door {
        /// Index of the goal door in `game_host.doors`.
        door_index: crate::gate::DoorIndex,
        /// The approach point (near side of the goal door).
        far_side_point: MapPoint,
        /// Far-side layer.
        far_side_layer: u16,
        /// True iff the goal-sector (far side) is a building.  When
        /// true the trailing step is a `CHANGE_POSITION` teleport after
        /// a random wait, not a plain walk to the far-side point.
        far_side_is_building: bool,
    },
    /// Line-goal.  The final MOVE uses the line's midpoint as its
    /// waypoint and carries `MoveFlags::LINE` + the line id so the
    /// actor's arrival check snaps to line tolerance.
    Line {
        /// Index of the goal line in `fast_grid.level.jump_lines`.
        line_index: crate::jump_line::JumpLineIndex,
        /// Midpoint of the line.  Used as the path target point during
        /// gate routing.
        midpoint: MapPoint,
        /// Arrival tolerance passed to the final line move.
        tolerance: f32,
    },
}

/// Timeout queue entry for a Move/Seek element whose pathfind failed.
/// When the pathfinder returns no path, the request is stamped with the
/// current universal frame counter and pushed onto this list.  After
/// 100 frames the element transitions to `Impossible` (and, for PCs,
/// the "unable to do something" speech line fires).
///
/// This is **not** a retry queue — the path is not re-dispatched during
/// the 100-frame window.  The element sits waiting (no orders, so the
/// actor's idle animation drives) until either the pathfinder produces
/// a result via external state changes, the element is cancelled (halt /
/// postpone), or the timeout elapses.
#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash,
)]
pub(crate) struct FailedPathRequest {
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    /// Universal frame counter at failure time.  Ages out at
    /// `first_fail_frame + 100`.
    pub(crate) first_fail_frame: u32,
}

/// Outcome of [`EngineInner::try_dispatch_move_path`], the unified
/// pathfind-and-populate pipeline invoked from the hourglass Move
/// dispatch.
#[derive(Debug)]
pub(crate) enum MovePathOutcome {
    /// Path found, orders populated, actor's `active_movement` + action
    /// state set, element transitioned to `InProgress`.  Caller has
    /// nothing left to do.
    Success,
    /// Pathfinder returned `None`.  The element has *not* been touched
    /// — caller enqueues it into `failed_path_requests` for the
    /// 100-frame timeout window.
    Failed,
    /// The entity slot is empty or the element vanished mid-dispatch.
    /// Caller should mark the element `Impossible`.
    ActorGone,
}

impl GoalShape {
    /// The point used for pathfinding / the final MOVE's destination.
    pub(crate) fn goal_point(&self) -> MapPoint {
        match *self {
            GoalShape::Point(p) => p,
            GoalShape::Seek { point, .. } => point,
            GoalShape::Door { far_side_point, .. } => far_side_point,
            GoalShape::Line { midpoint, .. } => midpoint,
        }
    }
}

/// Source adaptation when an actor is currently straddling a gate.
///
/// When the actor's current door is non-null, the path source is
/// rewritten to the gate's far-side point / sector / layer based on the
/// actor's door direction.
///
/// Returns `None` when the actor is not in a gate (callers should use
/// the raw `position_map` / `sector` / `layer`).
pub(crate) fn adapt_source_to_current_door(
    doors: &[crate::gate::Door],
    door_handle: crate::position_interface::DoorHandle,
    door_direction: bool,
) -> Option<(MapPoint, u16, u16)> {
    if door_handle.is_null() {
        return None;
    }
    let door = doors.get(door_handle.0 as usize)?;
    // door_direction true → use the "in" side of the door as the
    // source; false → use the "out" side.
    if door_direction {
        Some((
            MapPoint::new(door.point_in.0, door.point_in.1),
            u16::from(door.sector_in),
            door.layer_in,
        ))
    } else {
        Some((
            MapPoint::new(door.point_out.0, door.point_out.1),
            u16::from(door.sector_out),
            door.layer_out,
        ))
    }
}

/// Radius for circular dispatch (one third of [`GROUP_LIMIT_MAX`]).
const CIRCULAR_DISPATCH_RADIUS: f32 = 60.0;

/// Maximum centroid-to-member distance for mercenary formation to apply.
/// When any member is farther than this from the centroid, fall back to
/// circular dispatch.
const GROUP_LIMIT_MAX: f32 = 180.0;

fn force_sword_movement_for_sequence(seq: &mut crate::sequence::Sequence) {
    for elem in &mut seq.elements {
        if let crate::sequence::SequenceElementData::Movement { flags, .. } = &mut elem.data {
            *flags |= crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT;
        }
    }
}

/// Compute per-character destination points using circular distribution.
///
/// The circular dispatch fallback when the group is too spread out for
/// the mercenary formation.
///
/// Characters are arranged in a circle around `click_point`. Each
/// unassigned character picks the nearest available slot; when multiple
/// characters want the same slot, the one farthest from the click gets it
/// (the "worst placed" heuristic). The loop repeats until all characters
/// are assigned.
pub(crate) fn circular_dispatch_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    let n = pc_positions.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![click_point];
    }

    // Generate candidate positions in a circle around click_point.
    // Each candidate is `(0, -CIRCULAR_DISPATCH_RADIUS)` rotated by
    // `(i * TWO_PI / n)`.
    let candidates: Vec<MapPoint> = (0..n)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / n as f32;
            MapPoint::new(
                click_point.x + angle.sin() * CIRCULAR_DISPATCH_RADIUS,
                click_point.y - angle.cos() * CIRCULAR_DISPATCH_RADIUS,
            )
        })
        .collect();

    let mut result = vec![click_point; n];
    let mut assigned = vec![false; n];
    let mut candidate_taken = vec![false; candidates.len()];

    // Iterative assignment with conflict resolution.
    loop {
        // Each unassigned character picks its nearest untaken candidate.
        // Store (character_idx, sq_dist) per candidate.
        let mut claims: Vec<Vec<(usize, f32)>> = vec![Vec::new(); candidates.len()];

        for (ci, &pos) in pc_positions.iter().enumerate() {
            if assigned[ci] {
                continue;
            }
            let mut best_k = None;
            let mut best_d = f32::INFINITY;
            for (ki, &cand) in candidates.iter().enumerate() {
                if candidate_taken[ki] {
                    continue;
                }
                let dx = pos.x - cand.x;
                let dy = pos.y - cand.y;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best_k = Some(ki);
                }
            }
            if let Some(ki) = best_k {
                claims[ki].push((ci, best_d));
            }
        }

        let mut any_assigned = false;
        for (ki, claimants) in claims.iter().enumerate() {
            match claimants.len() {
                0 => {}
                1 => {
                    let (ci, _) = claimants[0];
                    result[ci] = candidates[ki];
                    assigned[ci] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
                _ => {
                    // Multiple characters want this candidate.
                    // Give it to the "worst-placed" claimant — the one
                    // whose distance to the contested slot is largest
                    // (per-claimant distance to the slot, not distance
                    // to the click point).
                    let worst = claimants
                        .iter()
                        .max_by(|(_, da), (_, db)| {
                            da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .unwrap()
                        .0;
                    result[worst] = candidates[ki];
                    assigned[worst] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
            }
        }

        if !any_assigned || assigned.iter().all(|&a| a) {
            break;
        }
    }

    // Any unassigned characters get the click point itself.
    result
}

/// Build the line-jump click-sequence shape:
///
/// 1. move to the selected source jump-line,
/// 2. execute the jump command from source to associated destination,
/// 3. move from the landing side to the original clicked point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_line_jump_click_sequence(
    owner: EntityId,
    action: OrderType,
    source_line_idx: crate::jump_line::JumpLineIndex,
    source_line: &crate::jump_line::JumpLine,
    destination_line_idx: crate::jump_line::JumpLineIndex,
    click_point: MapPoint,
    click_layer: u16,
    speed_factor: f32,
) -> crate::sequence::Sequence {
    use crate::element::Command;
    use crate::sequence::{
        Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
    };

    let mut seq = Sequence::new();

    let mut move_to_line = SequenceElement::new_movement(1, Command::Move, Some(owner), action);
    move_to_line.data = SequenceElementData::Movement {
        destination: source_line.get_middle_point(),
        layer: source_line.layer,
        sector: None,
        gate_id: None,
        line_id: Some(source_line_idx),
        element: None,
        flags: MoveFlags::LINE | MoveFlags::TO_JUMP,
        tolerance: 0.0,
        direction: 0,
        action,
        speed_factor,
        post_seek_sequence: None,
    };
    seq.append_element(move_to_line);

    let mut jump = SequenceElement::new_generic(2, Command::Jump, Some(owner));
    jump.set_property(Field::JumplineSource, FieldValue::LineId(source_line_idx));
    jump.set_property(
        Field::JumplineDestination,
        FieldValue::LineId(destination_line_idx),
    );
    seq.append_element(jump);

    let mut final_move = SequenceElement::new_movement(3, Command::Move, Some(owner), action);
    final_move.data = SequenceElementData::Movement {
        destination: click_point,
        layer: click_layer,
        sector: None,
        gate_id: None,
        line_id: None,
        element: None,
        flags: MoveFlags::empty(),
        tolerance: 0.0,
        direction: 0,
        action,
        speed_factor,
        post_seek_sequence: None,
    };
    seq.append_element(final_move);

    seq
}

impl EngineInner {
    pub(super) fn lift_endpoint_points(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> (GeoPoint2D, GeoPoint2D) {
        let sector = self
            .grid_sector_by_number(sector_number)
            .expect("DetermineMovementAnimation: missing lift sector");
        let low = sector.low_exit_point.unwrap_or_else(|| {
            panic!("DetermineMovementAnimation: lift sector {sector_number} missing low exit point")
        });
        let high = sector.high_exit_point.unwrap_or_else(|| {
            panic!(
                "DetermineMovementAnimation: lift sector {sector_number} missing high exit point"
            )
        });
        (low, high)
    }

    fn determine_lift_movement_animation(
        &self,
        owner: EntityId,
        posture_after: crate::element::Posture,
        action: OrderType,
        destination: MapPoint,
    ) -> OrderType {
        use crate::element::Posture;

        let Some(entity) = self.entities.get(owner.0 as usize).and_then(|e| e.as_ref()) else {
            return action;
        };
        let elem = entity.element_data();
        // legacy implementation `Instruct` stamps the current posture before
        // `DetermineMovementAnimation`. Some postponed Rust movement
        // elements can still be Undefined here, so use the same effective
        // actor posture instead of treating Undefined as upright.
        let posture = if posture_after == Posture::Undefined {
            elem.posture
        } else {
            posture_after
        };
        let Some(sector_handle) = elem.sector() else {
            return action;
        };
        let Some(sector) =
            self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector_handle)))
        else {
            return action;
        };
        let Some(lift_type) = sector.lift_type else {
            return action;
        };

        match posture {
            Posture::Upright => lift_type.translate_upright_action(action),
            Posture::OnWall | Posture::OnLadder => {
                if !matches!(
                    (posture, lift_type),
                    (Posture::OnWall, crate::sector::LiftType::Wall)
                        | (Posture::OnLadder, crate::sector::LiftType::Ladder)
                ) {
                    tracing::warn!(
                        ?owner,
                        ?posture,
                        ?lift_type,
                        sector = %sector.sector_number,
                        "DetermineMovementAnimation: climb posture does not match lift sector"
                    );
                    return action;
                }
                let (pt_low, pt_high) = self.lift_endpoint_points(sector.sector_number);
                let position = elem.position_map();
                let ladder_dx = pt_low.x - pt_high.x;
                let ladder_dy = pt_low.y - pt_high.y;
                let movement_dx = destination.x - position.x;
                let movement_dy = destination.y - position.y;
                let going_down = ladder_dx * movement_dx + ladder_dy * movement_dy >= 0.0;
                let translated = lift_type.translate_climb_action(action, going_down);
                tracing::debug!(
                    ?owner,
                    ?posture,
                    ?action,
                    ?translated,
                    sector = %sector.sector_number,
                    ladder_dx,
                    ladder_dy,
                    movement_dx,
                    movement_dy,
                    going_down,
                    "DetermineMovementAnimation: translated lift movement action"
                );
                translated
            }
            _ => action,
        }
    }

    pub(crate) fn apply_sword_movement_start_initiative_transfer(&mut self, entity_id: EntityId) {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());

        if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = false;
        }

        let Some(principal_id) = principal_id else {
            return;
        };
        let is_mutual = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return;
        }

        if let Some(Some(entity)) = self.entities.get_mut(principal_id.0 as usize)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }
    }

    fn maybe_provoke_after_sword_movement_terminated(
        &mut self,
        assets: &crate::engine::LevelAssets,
        entity_id: EntityId,
    ) {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());
        let Some(principal_id) = principal_id else {
            return;
        };

        let is_mutual = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return;
        }

        let Some(me) = self.get_entity(entity_id) else {
            return;
        };
        let Some(opponent) = self.get_entity(principal_id) else {
            return;
        };
        let me_pos = me.element_data().position();
        let opponent_pos = opponent.element_data().position();
        let dx = me_pos.x - opponent_pos.x;
        let dy = me_pos.y - opponent_pos.y;
        let dz = me_pos.z - opponent_pos.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();

        let Some(me_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(me, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return;
        };
        let Some(opponent_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(opponent, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return;
        };

        let between = |weapon: &crate::profiles::HtHWeaponProfile| {
            let lo = weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] as f32;
            let hi = weapon.distance[crate::weapons::WeaponDistance::Uber as usize] as f32;
            lo < distance && distance <= hi
        };
        if between(me_weapon) && between(opponent_weapon) {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Provoke,
                Some(entity_id),
            ));
        }
    }

    // ─── Order system ─────────────────────────────────────────────

    /// Snap a click/formation-slot point to the nearest authorized
    /// (walkable) position for a unit of the given size.
    ///
    /// Returns the adjusted point, or `None` if no walkable spot can be
    /// found near the click. Builds a move-box-sized bbox around the
    /// candidate point, pushes it away from any motion lines that would
    /// otherwise block the unit, then returns the box center.
    ///
    /// Without this snap, clicks that land on dynamic elements like
    /// drawbridges (whose surface lies just outside the static motion-area
    /// polygon) or even slightly inside an obstacle's bbox fail
    /// `object_position_authorized` and the pathfinder refuses to build
    /// a path — so the click appears to do nothing.
    ///
    /// `reference` is used as the "push toward" anchor — typically the
    /// raw click point passed alongside the per-PC formation slot.
    ///
    /// Callers must skip this snap when the click hits a Door/Drawbridge
    /// sector — the cross-sector gate A* path routes the PC through the
    /// door's entry point, which is the only walkable approach when the
    /// door sector itself isn't a motion area (e.g. a raised drawbridge).
    pub fn snap_click_to_walkable(
        &self,
        candidate: MapPoint,
        reference: MapPoint,
        layer: u16,
        half_diagonal_idx: usize,
    ) -> Option<MapPoint> {
        let hd = self
            .fast_grid
            .level
            .move_box_half_diagonals
            .get(half_diagonal_idx)
            .copied()?;
        let mut bbox = crate::geo2d::BBox2D::from_corners(
            crate::geo2d::pt(candidate.x - hd.x, candidate.y - hd.y),
            crate::geo2d::pt(candidate.x + hd.x, candidate.y + hd.y),
        );
        if self
            .fast_grid
            .find_authorized_position_toward(&mut bbox, reference, layer)
        {
            Some(MapPoint::from_geo(bbox.center()))
        } else {
            None
        }
    }

    /// Issue movement orders for a group of selected PCs around a single
    /// click point.
    ///
    /// Uses the "mercenary" formation: each PC keeps its position
    /// relative to the group centroid and walks to the corresponding
    /// offset around `click_point`.  The marker for each PC is placed
    /// at *its own* resolved destination, not at the raw click point.
    ///
    /// Each per-PC formation slot is then snapped to a walkable spot via
    /// [`EngineInner::snap_click_to_walkable`].  This is what allows
    /// clicks on drawbridges and other dynamic elements to actually move
    /// PCs onto them — the raw click often lands just outside the
    /// walkable polygon, and the snap pulls it back inside.
    ///
    /// Uses mercenary formation for compact groups and circular dispatch
    /// for spread-out groups.
    pub(crate) fn perform_group_move(
        &mut self,
        assets: &LevelAssets,
        pc_ids: &[EntityId],
        click_point: MapPoint,
        run: bool,
        show_marker: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
    ) {
        if pc_ids.is_empty() {
            return;
        }

        // Preemption is handled downstream by `arbitrate_instruct`:
        // every same-sector PC gets a fresh `Command::Move` sequence
        // element launched via `launch_element` below, which reaches
        // `InstructOwner` on the next hourglass and drives the standard
        // priority-arbitration cascade.  A pending scroll/object pickup
        // (Seek + queued Take at `Normal`) vs a new Move at `Normal`
        // resolves to `InterruptCurrent`, which cleanly tears down both
        // the seek and its post-seek Take via the `NEXT_LEVEL` cascade.
        // Earlier fixes tried to short-circuit this with explicit
        // `stop_owner` calls, but `stop_owner` on a movement element
        // keeps the element InProgress "for transition", which left the
        // stale seek hanging when the same-sector shortcut was
        // direct-pathfinder rather than a proper Move element.

        // Collect each PC's current map position, layer, and sector.
        let positions: Vec<(EntityId, MapPoint, u16, u16)> = pc_ids
            .iter()
            .filter_map(|&pc_id| {
                self.get_entity(pc_id).map(|e| {
                    let elem = e.element_data();
                    (
                        pc_id,
                        elem.position_map(),
                        elem.layer(),
                        elem.sector().map(u16::from).unwrap_or(0),
                    )
                })
            })
            .collect();
        if positions.is_empty() {
            return;
        }

        let src_layer = positions[0].2;
        let reference = positions[0].1;

        // ── Unified sector hit-test ──
        //
        // Top-down layer search to set the selected sector / layer /
        // valid-for-move flags.  When `goal_override` is set (patch
        // click), skip the spatial query: the proto-loaded
        // `(patch.sector, patch.layer)` are authoritative.  This mirrors
        // C++ where `update_mouse` substitutes
        // `mpSelectedSector = mFastGrid.GetSector(patch.sector)` and
        // `muwSelectedLayer = patch.layer` before `PerformGroupMove`
        // reads them at `RHengine.cpp:5301-5306`.
        let (
            goal_sector,
            effective_click,
            effective_layer,
            is_valid,
            is_door_click,
            is_jump_click,
            clicked_door_index,
        ) = if let Some((override_sector, override_layer)) = goal_override {
            // Patch-click path: route to (patch.sector, patch.layer)
            // unconditionally.  The waypoint becomes the destination
            // without snapping; per-PC routing below still calls
            // `snap_click_to_walkable` at `effective_layer` to find a
            // valid landing position.  Patches are never doors / jumps,
            // so those shortcuts stay false.  `is_valid = true` keeps
            // the simple-move branch reachable when the PC is already
            // in `patch.sector`.
            (
                Some(override_sector),
                click_point,
                override_layer,
                true,
                false,
                false,
                None,
            )
        } else {
            let hit = self.fast_grid.get_sector_screen(click_point, reference);
            let is_valid = hit.is_valid_for_move(&self.fast_grid);

            // ── Door/Drawbridge click shortcut ──
            //
            // When the click hits a door sector, bypass the walkability
            // snap on formation slots.  Per-PC routing must also skip
            // `snap_click_to_walkable` so the destination stays in the
            // door sector and the gate-A* path routes through the
            // door's entry point (the door sector itself is not a
            // motion area).
            let is_door_click_sector = hit
                .sector_idx
                .and_then(|i| self.fast_grid.level.sectors.get(usize::from(i)))
                .is_some_and(|s| s.sector_type.is_door());
            let is_jump_click = hit
                .sector_idx
                .and_then(|i| self.fast_grid.level.sectors.get(usize::from(i)))
                .is_some_and(|s| s.sector_type.is_jump());

            // Door index of the clicked door sector, if any.  Used to
            // route the per-PC gate search via `find_path_to_door` and
            // emit a `GoalShape::Door` terminal element.
            let clicked_sector_door_index: Option<u32> = hit
                .sector_idx
                .and_then(|i| self.fast_grid.level.sectors.get(usize::from(i)))
                .and_then(|s| s.door_index);
            let clicked_polygon_door_index = self
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .and_then(|h| door_click_polygon_at(h, click_point));
            let clicked_door_index = clicked_sector_door_index.or(clicked_polygon_door_index);
            let is_door_click = is_door_click_sector || clicked_door_index.is_some();

            let (effective_click, effective_layer) = if is_valid {
                (click_point, hit.layer)
            } else {
                let snapped = self.snap_to_nearest_walkable(assets, click_point, src_layer);
                (snapped.unwrap_or(click_point), src_layer)
            };
            (
                hit.sector,
                effective_click,
                effective_layer,
                is_valid,
                is_door_click,
                is_jump_click,
                clicked_door_index,
            )
        };

        // ── Compute formation slots around the click point ──
        //
        // If the group is compact enough, use mercenary formation
        // (preserve relative positions).  Otherwise use circular
        // dispatch (arrange in a circle around click).
        let pc_positions: Vec<MapPoint> = positions.iter().map(|(_, p, _, _)| *p).collect();
        let dests = {
            let n = pc_positions.len() as f32;
            let cx = pc_positions.iter().map(|p| p.x).sum::<f32>() / n;
            let cy = pc_positions.iter().map(|p| p.y).sum::<f32>() / n;
            let max_sq_dist = pc_positions
                .iter()
                .map(|p| {
                    let dx = p.x - cx;
                    let dy = p.y - cy;
                    dx * dx + dy * dy
                })
                .fold(0.0f32, f32::max);
            if max_sq_dist <= GROUP_LIMIT_MAX * GROUP_LIMIT_MAX {
                mercenary_formation_destinations(&pc_positions, effective_click)
            } else {
                // Snap each circular candidate to a walkable position
                // before the assignment algorithm.  Skip the snap on
                // door clicks so the raw formation slot feeds into the
                // cross-sector gate-A* path below.
                let raw = circular_dispatch_destinations(&pc_positions, effective_click);
                if is_door_click {
                    raw
                } else {
                    raw.into_iter()
                        .map(|c| {
                            self.snap_click_to_walkable(c, effective_click, effective_layer, 0)
                                .unwrap_or(c)
                        })
                        .collect()
                }
            }
        };

        // ── Per-PC routing ──
        // For each PC, decide between:
        //   1. Same-sector: simple MOVE
        //   2. Cross-sector (door/lift): gate-A* sequence
        for ((pc_id, _, _, src_sector), dest) in positions.iter().zip(dests.iter()) {
            if is_jump_click {
                let pc_pos = positions
                    .iter()
                    .find(|(id, _, _, _)| *id == *pc_id)
                    .map(|(_, p, _, _)| *p)
                    .unwrap_or(*dest);
                let source_line_idx = self.get_nearest_jumpable_jump_line(
                    *pc_id,
                    pc_pos.to_geo(),
                    effective_click.to_geo(),
                    false,
                );
                let Some(source_line_idx) =
                    source_line_idx.and_then(crate::jump_line::JumpLineIndex::new)
                else {
                    tracing::error!(
                        actor = ?pc_id,
                        click_x = effective_click.x,
                        click_y = effective_click.y,
                        "line-jump click hit a jump sector but no executable jump line was found",
                    );
                    self.hero_speaking(
                        assets,
                        *pc_id,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    continue;
                };
                let Some(source_line) = self
                    .fast_grid
                    .level
                    .jump_lines
                    .get(usize::from(source_line_idx))
                    .cloned()
                else {
                    panic!("line-jump source line {source_line_idx} is missing");
                };
                let Some(destination_line_idx) = source_line
                    .associated_line_index
                    .and_then(crate::jump_line::JumpLineIndex::new)
                else {
                    panic!("line-jump source line {source_line_idx} has no associated line");
                };
                if self
                    .fast_grid
                    .level
                    .jump_lines
                    .get(usize::from(destination_line_idx))
                    .is_none()
                {
                    panic!(
                        "line-jump destination line {destination_line_idx} for source {source_line_idx} is missing"
                    );
                }

                let is_swordfighting = self
                    .get_entity(*pc_id)
                    .and_then(|e| e.human_data())
                    .map(|h| !h.opponents.is_empty())
                    .unwrap_or(false);
                let action = if is_swordfighting {
                    let want = if run {
                        OrderType::RunningWithSword
                    } else {
                        OrderType::WalkingWithSword
                    };
                    let has_sword_row = self
                        .get_entity(*pc_id)
                        .map(|e| e.sprite().has_animation(want))
                        .unwrap_or(false);
                    if has_sword_row {
                        want
                    } else if run {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    }
                } else if run {
                    OrderType::RunningUpright
                } else {
                    match self.get_entity(*pc_id).map(|e| e.element_data().posture) {
                        Some(crate::element::Posture::Crouched) => OrderType::WalkingCrouched,
                        _ => OrderType::WalkingUpright,
                    }
                };

                let mut seq = build_line_jump_click_sequence(
                    *pc_id,
                    action,
                    source_line_idx,
                    &source_line,
                    destination_line_idx,
                    effective_click,
                    effective_layer,
                    1.0,
                );
                if is_swordfighting {
                    force_sword_movement_for_sequence(&mut seq);
                }
                let speak = crate::sequence::SequenceElement::new(
                    4,
                    crate::element::Command::SpeakHeroReachDestination,
                    Some(*pc_id),
                );
                seq.append_element(speak);
                self.append_posture_recovery(*pc_id, &mut seq);
                self.launch_sequence(seq);
                if show_marker && !is_door_click {
                    self.ground_mark.add_mark(
                        effective_click.x,
                        effective_click.y,
                        effective_layer,
                    );
                }
                continue;
            }

            // Same-sector or unknown goal sector: simple move
            if !is_door_click
                && (!is_valid
                    || goal_sector.is_none()
                    || goal_sector.is_some_and(|goal| u16::from(goal) == *src_sector))
            {
                // Door clicks skip the walkable snap entirely.
                let snap_res = if is_door_click {
                    Some(*dest)
                } else {
                    self.snap_click_to_walkable(*dest, effective_click, effective_layer, 0)
                };
                let snapped = match snap_res {
                    Some(pt) => pt,
                    None => {
                        // FindAuthorizedPosition failure on the
                        // mercenary/same-sector path fires
                        // HERO_UNABLE_TO_DO_SOMETHING and skips the
                        // move for this PC.
                        self.hero_speaking(
                            assets,
                            *pc_id,
                            crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                        );
                        continue;
                    }
                };
                // Launch a Move sequence element.  Going through the
                // sequence pipeline — rather than a direct
                // `pathfinder.add_request` shortcut — means the element
                // hits `arbitrate_instruct` when it transitions
                // Todo → InProgress next hourglass.  Any pending Seek +
                // post-seek Take (from a prior scroll-pickup click) at
                // Normal priority is interrupted by the new Normal Move
                // via the NEXT_LEVEL cascade, cleanly tearing down the
                // pickup so it doesn't replay at the new destination.
                let is_swordfighting = self
                    .get_entity(*pc_id)
                    .and_then(|e| e.human_data())
                    .map(|h| !h.opponents.is_empty())
                    .unwrap_or(false);
                let action = if is_swordfighting {
                    // Sword-variant pick with PC-fallback.  When the
                    // determined sword animation isn't in the actor's
                    // sprite profile, fall back to the plain upright
                    // variant.  PC sprites (e.g. "Robin des bois") ship
                    // without `WalkingWithSword` / `RunningWithSword`
                    // rows — the dev-mode assert compiles out in
                    // release, leaving the upright fallback as the
                    // shipping behaviour.
                    let want = if run {
                        OrderType::RunningWithSword
                    } else {
                        OrderType::WalkingWithSword
                    };
                    let has_sword_row = self
                        .get_entity(*pc_id)
                        .map(|e| e.sprite().has_animation(want))
                        .unwrap_or(false);
                    if has_sword_row {
                        want
                    } else if run {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    }
                } else if run {
                    OrderType::RunningUpright
                } else {
                    match self.get_entity(*pc_id).map(|e| e.element_data().posture) {
                        Some(crate::element::Posture::Crouched) => OrderType::WalkingCrouched,
                        _ => OrderType::WalkingUpright,
                    }
                };
                let mut move_elem = crate::sequence::SequenceElement::new_movement(
                    1,
                    crate::element::Command::Move,
                    Some(*pc_id),
                    action,
                );
                if let crate::sequence::SequenceElementData::Movement {
                    destination,
                    layer,
                    flags,
                    ..
                } = &mut move_elem.data
                {
                    *destination = snapped;
                    *layer = effective_layer;
                    if is_swordfighting {
                        *flags |= crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT;
                    }
                }

                // Append a `SpeakHeroReachDestination` element after
                // the move and cap the sequence with any
                // posture-cleanup sub-elements the PC needs (re-equip
                // bow, re-crouch, re-enter HelpingClimb / Beggar,
                // demote trailing ShootBow to ShootBowOnce).  The PC's
                // `Instruct` override terminates the Speak element on
                // dispatch and queues the HERO_DONE_COMMAND bark
                // (handled by `arbitrate_instruct`).
                let speak = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::SpeakHeroReachDestination,
                    Some(*pc_id),
                );
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(move_elem);
                seq.append_element(speak);
                self.append_posture_recovery(*pc_id, &mut seq);
                self.launch_sequence(seq);
                if show_marker && !is_door_click {
                    self.ground_mark
                        .add_mark(snapped.x, snapped.y, effective_layer);
                }
                continue;
            }

            if goal_sector.is_none() && !is_door_click {
                tracing::warn!("skipping cross-sector move without resolved goal sector");
                continue;
            };

            // Cross-sector: try gate A*
            let pc_pos_raw = positions
                .iter()
                .find(|(id, _, _, _)| *id == *pc_id)
                .map(|(_, p, _, _)| *p)
                .unwrap_or(*dest);

            // Source adaptation: if the PC is currently straddling a
            // gate, use the gate's far-side point / sector as the path
            // source.  Without this, the pathfinder starts from inside
            // the door sector, which is not a motion area and yields no
            // valid seed gates.
            let (door_handle, door_direction) = self
                .get_entity(*pc_id)
                .map(|e| e.position_iface())
                .map(|p| (p.get_door(), p.get_door_direction()))
                .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
            let (pc_pos, path_src_sector, _path_src_layer) = {
                let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
                let adapted = host.and_then(|h| {
                    adapt_source_to_current_door(&h.doors, door_handle, door_direction)
                });
                match adapted {
                    Some((adj, sector, layer)) => (adj, sector, layer),
                    None => (pc_pos_raw, *src_sector, src_layer),
                }
            };

            // Door-click routing: when the click lands on a door
            // sector with a known `door_index`, use
            // `find_path_to_door` and `GoalShape::Door` so the trailing
            // emission walks the PC up to the door's near-side (and
            // CHANGE_POSITION-teleports into buildings, turns the PC to
            // face the lock for lockpicks, etc.).
            let door_goal = if is_door_click {
                clicked_door_index
            } else {
                None
            };

            // PC authorisation for the gate A*.  Click-to-move never
            // sets `MoveFlags::MAP`, so `allow_leave_map = false` here.
            let pc_auth = self.get_entity(*pc_id).map(|e| e.actor_auth_info());
            let door_goal_info = door_goal.and_then(|door_idx| {
                let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
                host.and_then(|h| {
                    crate::gate::find_path_to_door(
                        &h.doors,
                        (pc_pos.x, pc_pos.y),
                        path_src_sector,
                        crate::gate::DoorIndex(door_idx),
                        pc_auth.as_ref(),
                        false,
                        &|sector| {
                            h.sector_kinds
                                .get(&u16::from(sector))
                                .and_then(|k| k.lift_type)
                        },
                    )
                    .map(|(path, pt, sector, layer)| (door_idx, path, pt, sector, layer))
                })
            });

            let door_far_side_is_building = door_goal_info.as_ref().map(|(_, _, _, sector, _)| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(*sector as i16))
                    .map(|gs| gs.sector_type.is_building())
                    .unwrap_or(false)
            });

            let path = if door_goal_info.is_some() {
                door_goal_info.as_ref().map(|(_, p, _, _, _)| p.clone())
            } else {
                let Some(goal_sector) = goal_sector else {
                    tracing::warn!("skipping gate path without resolved goal sector");
                    continue;
                };
                let game_host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
                game_host.and_then(|h| {
                    crate::gate::find_path_gates(
                        &h.doors,
                        (pc_pos.x, pc_pos.y),
                        path_src_sector,
                        (effective_click.x, effective_click.y),
                        u16::from(goal_sector),
                        pc_auth.as_ref(),
                        false,
                        &|sector| {
                            h.sector_kinds
                                .get(&u16::from(sector))
                                .and_then(|k| k.lift_type)
                        },
                    )
                })
            };

            match path {
                Some(gate_steps) => {
                    tracing::info!(
                        "Gate A* from sector {} to sector {}: {} gates{}",
                        src_sector,
                        goal_sector.map(u16::from).unwrap_or(*src_sector),
                        gate_steps.len(),
                        if door_goal.is_some() {
                            " (door goal)"
                        } else {
                            ""
                        },
                    );
                    let goal_shape = if let Some((door_idx, _, pt, _sector, layer)) = door_goal_info
                    {
                        GoalShape::Door {
                            door_index: crate::gate::DoorIndex(door_idx),
                            far_side_point: MapPoint::new(pt.0, pt.1),
                            far_side_layer: layer,
                            far_side_is_building: door_far_side_is_building.unwrap_or(false),
                        }
                    } else {
                        GoalShape::Point(*dest)
                    };
                    self.build_gate_movement_sequence(
                        *pc_id,
                        gate_steps,
                        goal_shape,
                        effective_layer,
                        if run {
                            OrderType::RunningUpright
                        } else {
                            OrderType::WalkingUpright
                        },
                        true,
                        1.0,
                        if self
                            .get_entity(*pc_id)
                            .and_then(|e| e.human_data())
                            .map(|h| !h.opponents.is_empty())
                            .unwrap_or(false)
                        {
                            crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT
                        } else {
                            crate::sequence::MoveFlags::empty()
                        },
                        Vec::new(),
                        Vec::new(),
                        true,
                        true,
                    );
                    if show_marker && !is_door_click {
                        self.ground_mark.add_mark(dest.x, dest.y, effective_layer);
                    }
                }
                _ => {
                    // No gate path — fall back to direct move.  On a
                    // door click the raw formation slot stands in for
                    // the snap result.
                    let snap_res = if is_door_click {
                        Some(*dest)
                    } else {
                        self.snap_click_to_walkable(*dest, effective_click, effective_layer, 0)
                    };
                    let snapped = match snap_res {
                        Some(pt) => pt,
                        None => {
                            // FindAuthorizedPosition failure on the
                            // circular/fallback path fires
                            // HERO_UNABLE_TO_DO_SOMETHING and skips the PC.
                            self.hero_speaking(
                                assets,
                                *pc_id,
                                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                            );
                            continue;
                        }
                    };
                    self.issue_move_order(*pc_id, snapped, run);
                    if show_marker && !is_door_click {
                        self.ground_mark
                            .add_mark(snapped.x, snapped.y, effective_layer);
                    }
                }
            }
        }

        // At the tail of group-move, if the click happened during
        // macro recording the messenger forwards `StopRecordingMacro`.
        // Routing through the messenger keeps the downstream
        // bookkeeping (QA HUD reset, macro-slot commit) consistent
        // with other stop points.
        if self.is_recording_macro() {
            self.messenger.send(crate::messenger::Message::pc(
                crate::messenger::PcMessage::StopRecordingMacro,
                None,
            ));
        }
    }

    /// Search concentric rings for the nearest point inside a walkable
    /// motion area polygon on the given layer. Used when a click lands
    /// outside all sectors.
    fn snap_to_nearest_walkable(
        &self,
        assets: &LevelAssets,
        click: MapPoint,
        layer: u16,
    ) -> Option<MapPoint> {
        for radius_step in 1..=20u32 {
            let r = radius_step as f32 * 10.0;
            for dir in 0..16u32 {
                let angle = dir as f32 * std::f32::consts::FRAC_PI_8;
                let candidate = MapPoint::new(click.x + angle.sin() * r, click.y - angle.cos() * r);
                if assets
                    .pathfinder_graph
                    .find_area_at_point(layer as usize, candidate)
                    .is_some()
                {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Build a movement sequence that traverses a gate path from
    /// `find_path_gates` and ends at `goal` on `goal_layer`.
    ///
    /// Unifies the three goal shapes (point, door, line) — callers
    /// pick via [`GoalShape`].
    ///
    /// For each gate in the path the emitted sub-elements depend on
    /// the gate's type:
    ///
    /// * **Jump gates** emit a single `Jump` element carrying the
    ///   source / destination `JumpLine` indices.  The tick handler
    ///   consumes those via [`EngineInner::start_jump`].
    /// * **Building doors** (previous sector `is_building()` true)
    ///   emit `WaitTimer(50)` (skipped on the first gate),
    ///   `WaitTimer(rand & 15 + rand & 15)`, and `ChangePosition` to
    ///   the gate's outside point.  The wait + teleport drives the
    ///   "actor walks inside the building and re-emerges" illusion.
    /// * **Regular doors** emit `Move` to the gate's entry point
    ///   followed by `AssertPosition` that the actor reached it.
    ///
    /// After the approach sub-elements, the door itself is crossed:
    ///
    /// * A **locked PC door** that the PC can pick (`unlockable` +
    ///   `has_lockpick`) emits `Turn` toward the lock then
    ///   `UnlockDoor` and *returns* — the door is expected to re-issue
    ///   the move once the lockpick animation terminates.
    /// * Ladder-lift sectors interpose a `WaitFreeLift` before
    ///   `PassDoor` so the climber waits for the ladder to free up.
    /// * All other doors emit `PassDoor` + `AssertPosition`.
    ///
    /// Trailing emission depends on [`GoalShape`]:
    ///
    /// * **Point goal** — emit a plain `Move` to the goal point
    ///   unless the last gate dropped the actor into a building
    ///   sector.  Skipped entirely when `move_after_last_door` is
    ///   `false` (the "walk up to the door" variant).
    /// * **Door goal** — emit the building CHANGE_POSITION or plain
    ///   MOVE to the far-side point of the goal door, then optionally
    ///   TURN + UNLOCK_DOOR for PC-lockable goal doors.
    /// * **Line goal** — emit a plain `Move` to the line's midpoint
    ///   carrying `MoveFlags::LINE` and the line id so the actor's
    ///   arrival check snaps to line tolerance.  Intermediate gate
    ///   moves never carry `MoveFlags::LINE`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_gate_movement_sequence(
        &mut self,
        entity_id: EntityId,
        gate_path: Vec<crate::gate::GatePathStep>,
        goal: GoalShape,
        goal_layer: u16,
        base_action: OrderType,
        move_after_last_door: bool,
        speed_factor: f32,
        initial_flags: crate::sequence::MoveFlags,
        prefix_elements: Vec<crate::sequence::SequenceElement>,
        tail_elements: Vec<crate::sequence::SequenceElement>,
        append_arrival_speech: bool,
        append_recovery: bool,
    ) {
        use crate::element::Command;
        use crate::sequence::{
            Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
        };

        // Determine first jump gate.  Every gate *before* the first
        // jump gets the `TO_JUMP` flag so its movement element sets
        // the actor up for the jump.  Only the Point and Door goal
        // variants apply this flag-mutation; the Line variant passes
        // the input flags to every gate unmodified, so suppress the
        // OR for `GoalShape::Line`.
        let apply_to_jump = !matches!(goal, GoalShape::Line { .. });
        let first_jump: Option<usize> = if apply_to_jump {
            gate_path.iter().enumerate().find_map(|(i, step)| {
                let host = self.mission_script.as_ref().and_then(|s| s.game_host());
                let is_jump = host
                    .and_then(|h| h.doors.get(usize::from(step.door_index)))
                    .map(|d| d.is_jump())
                    .unwrap_or(false);
                if is_jump { Some(i) } else { None }
            })
        } else {
            None
        };

        let flags_at = |gate_idx: usize| -> MoveFlags {
            match first_jump {
                Some(j) if gate_idx <= j => initial_flags | MoveFlags::TO_JUMP,
                _ => initial_flags,
            }
        };

        // Snapshot all the gate data we need while we briefly hold
        // the GameHost borrow, so the main loop can call grid /
        // sequence helpers on `self` without fighting the borrow
        // checker.
        #[derive(Clone, Copy)]
        struct GateShot {
            door_index: crate::gate::DoorIndex,
            direct: bool,
            // Geometry used by the emitted sub-elements.
            entry: MapPoint,
            exit: MapPoint,
            entry_layer: u16,
            exit_layer: u16,
            // Where the actor ends up *after* crossing.
            new_sector: u16,
            // Gate typing.
            is_jump: bool,
            jump_line_src: Option<crate::jump_line::JumpLineIndex>,
            jump_line_dst: Option<crate::jump_line::JumpLineIndex>,
            // Door typing (only meaningful when !is_jump).
            is_locked_pc_unlockable: bool,
            // Original RHsequence.cpp keeps the caller's action on
            // gate approach, WAIT_FREE_LIFT, PASS_DOOR, and post-pass
            // asserts.  Door-specific GetAction1/2 calls exist in
            // original-code but are commented out at execution time.
            entry_action: OrderType,
            door_action: OrderType,
        }

        let (gate_shots, starting_sector) = {
            let game_host = match self.mission_script.as_mut().and_then(|s| s.game_host_mut()) {
                Some(h) => h,
                None => return,
            };
            // Starting sector = the sector on the "old" side of the
            // first gate.  Needed to decide whether the actor's
            // current location is a building (→ first gate uses the
            // CHANGE_POSITION branch) or not.
            let start_sector = gate_path
                .first()
                .and_then(|first| game_host.doors.get(usize::from(first.door_index)))
                .map(|d| {
                    if gate_path[0].direct {
                        u16::from(d.sector_out)
                    } else {
                        u16::from(d.sector_in)
                    }
                });
            let shots: Vec<GateShot> = gate_path
                .iter()
                .filter_map(|step| {
                    let door = game_host.doors.get(usize::from(step.door_index))?;
                    let (entry, exit, entry_layer, exit_layer, new_sector) = if step.direct {
                        (
                            MapPoint::new(door.point_out.0, door.point_out.1),
                            MapPoint::new(door.point_in.0, door.point_in.1),
                            door.layer_out,
                            door.layer_in,
                            u16::from(door.sector_in),
                        )
                    } else {
                        (
                            MapPoint::new(door.point_in.0, door.point_in.1),
                            MapPoint::new(door.point_out.0, door.point_out.1),
                            door.layer_in,
                            door.layer_out,
                            u16::from(door.sector_out),
                        )
                    };
                    let is_jump = door.is_jump();
                    let (jump_src, jump_dst) = if is_jump {
                        let (s, d) = if step.direct {
                            (door.jump_line_out, door.jump_line_in)
                        } else {
                            (door.jump_line_in, door.jump_line_out)
                        };
                        (
                            s.and_then(crate::jump_line::JumpLineIndex::new),
                            d.and_then(crate::jump_line::JumpLineIndex::new),
                        )
                    } else {
                        (None, None)
                    };
                    let is_locked_pc_unlockable = !is_jump && door.locked_pc && door.unlockable;
                    let (entry_action, door_action) = (base_action, base_action);
                    Some(GateShot {
                        door_index: step.door_index,
                        direct: step.direct,
                        entry,
                        exit,
                        entry_layer,
                        exit_layer,
                        new_sector,
                        is_jump,
                        jump_line_src: jump_src,
                        jump_line_dst: jump_dst,
                        is_locked_pc_unlockable,
                        entry_action,
                        door_action,
                    })
                })
                .collect();
            (shots, start_sector)
        }; // host borrow dropped here

        if gate_shots.is_empty() {
            return;
        }

        // Does the entity have the lockpick contextual action?
        // Needed to choose the lockpick sub-element branch.
        let has_lockpick = self
            .get_entity(entity_id)
            .map(|e| e.actor_auth_info().has_lockpick)
            .unwrap_or(false);

        // Resolve sector → is_building via the fast grid.  Returns
        // false for unknown sectors (treat motion areas as
        // non-building).
        let is_building_sector = |this: &Self, sector: u16| -> bool {
            this.grid_sector_by_number(crate::sector::SectorNumber::new(sector as i16))
                .map(|gs| gs.sector_type.is_building())
                .unwrap_or(false)
        };

        let is_ladder_lift = |this: &Self, sector: u16| -> bool {
            this.grid_sector_by_number(crate::sector::SectorNumber::new(sector as i16))
                .and_then(|gs| gs.lift_type)
                .map(|lt| lt == crate::sector::LiftType::Ladder)
                .unwrap_or(false)
        };

        let mut seq = Sequence::new();
        let mut level: u16 = 1;

        for mut elem in prefix_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Track the "previous" sector so each gate knows what it's
        // coming *from*.  After the first gate, this is the
        // previous gate's `new_sector`.
        let mut prev_sector: Option<u16> = starting_sector;

        // Cross-sector source-sector sanity assert.  When the goal
        // sector differs from the source, prepend an `AssertPosition`
        // against the source sector so the actor's location is
        // re-validated right before the gate walk begins; if the actor
        // was nudged out between scheduling and dispatch the sequence
        // aborts gracefully instead of following a stale path.  This
        // unified builder is only invoked for cross-sector traversals
        // (callers handle same-sector inline), so emit unconditionally.
        if let Some(src_sector) = starting_sector {
            let mut leading_ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                base_action,
            );
            leading_ap.data = SequenceElementData::Movement {
                destination: crate::coordinates::MapPoint::default(),
                layer: 0,
                sector: crate::position_interface::SectorHandle::new(src_sector),
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                // Original leading cross-sector ASSERT_POSITION uses
                // the constructor default tolerance (0).  Gate-entry
                // and post-pass asserts explicitly pass 10.
                tolerance: 0.0,
                direction: 0,
                action: base_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(leading_ap);
            level += 1;
        }

        let seek_goal = match goal {
            GoalShape::Seek {
                target, tolerance, ..
            } => Some((target, tolerance)),
            _ => None,
        };

        // Goal-point used for the trailing MOVE (if any).  For Point
        // goals this is the caller's point; for Line goals it's the
        // line's midpoint; for Door goals it's the approach point on
        // the near side of the goal door.
        let goal_point = goal.goal_point();

        // Tracks whether a lockpick branch on an intermediate gate
        // terminated the sequence early.
        let mut ended_early = false;

        // Element count captured after the leading AssertPosition (if
        // any) and used by the building-source branch to skip the
        // 50-frame WaitTimer on the first gate's emission.
        let first_gate_element_count = seq.elements.len();

        for (gate_idx, shot) in gate_shots.iter().enumerate() {
            let gate_flags = flags_at(gate_idx);
            // -------- Gate approach branch --------
            //
            // Doors and jumps both first move to the gate's source
            // point (or CHANGE_POSITION out of a building), matching
            // original AppendMoveToSequence.  The door-vs-jump split
            // happens after this approach.
            let old_is_building = prev_sector
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);

            // Original sequence construction uses the caller's action
            // for both approach and door-pass sub-elements.
            let entry_action = shot.entry_action;
            let door_action = shot.door_action;

            if old_is_building {
                // When the previous sector is a building, the actor
                // "walks inside" by waiting out a timer then
                // teleporting to the gate's outside point.  Two
                // WaitTimer elements: the 50-frame one is only added
                // when there was already a prior gate-emitted element
                // (so the very first gate skips it).
                let wait_command = if matches!(goal, GoalShape::Line { .. }) {
                    Command::Wait
                } else {
                    Command::WaitTimer
                };
                if seq.elements.len() != first_gate_element_count {
                    let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                    w.set_property(Field::Timer, FieldValue::Integer(50));
                    seq.append_element(w);
                    level += 1;
                }
                // Random 0..30 (sum of two rand-and-15 draws).
                let r: u32 = self.rng.u32(0..16) + self.rng.u32(0..16);
                let mut w = SequenceElement::new_generic(level, wait_command, Some(entity_id));
                w.set_property(Field::Timer, FieldValue::Integer(r));
                seq.append_element(w);
                level += 1;

                // CHANGE_POSITION — instant teleport to the gate's
                // "outside" point (the `entry` in our direction).
                // Compute a 0..15 direction from (exit - entry) so
                // the sprite is facing the exit.  We stuff that into
                // the element's direction field for the tick handler
                // to apply.
                let dx = shot.exit.x - shot.entry.x;
                let dy = shot.exit.y - shot.entry.y;
                let dir = crate::position_interface::vector_to_sector_0_to_15(dx, dy);
                let mut cp = SequenceElement::new_movement(
                    level,
                    Command::ChangePosition,
                    Some(entity_id),
                    entry_action,
                );
                cp.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: shot.entry_layer,
                    // Assert actor is still in the building sector
                    // before teleporting.  Building teleport is an
                    // in-sector position change, not a door-pass, so
                    // no gate ref is attached.
                    sector: prev_sector.and_then(crate::position_interface::SectorHandle::new),
                    gate_id: None,
                    line_id: None,
                    element: None,
                    flags: gate_flags,
                    tolerance: 0.0,
                    direction: dir,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(cp);
                level += 1;
            } else {
                // MOVE to gate entry point on the source side.
                let gate_seek_target = seek_goal.map(|(target, _)| target);
                let mut m = SequenceElement::new_movement(
                    level,
                    Command::Move,
                    Some(entity_id),
                    entry_action,
                );
                m.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    // Original gate-approach MOVE uses the plain
                    // point+victim constructor and does not SetGate;
                    // only WAIT_FREE_LIFT/PASS_DOOR carry the gate.
                    gate_id: None,
                    line_id: None,
                    element: gate_seek_target,
                    flags: gate_flags,
                    // Original AppendMoveToSequence passes the
                    // seek victim through to gate-approach moves but
                    // uses tolerance 0; fTolerance belongs to the
                    // final goal/seek move.
                    tolerance: 0.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(m);
                level += 1;

                // ASSERT_POSITION that the actor actually reached
                // the gate.  Tolerance is 10.
                let mut ap = SequenceElement::new_movement(
                    level,
                    Command::AssertPosition,
                    Some(entity_id),
                    entry_action,
                );
                ap.data = SequenceElementData::Movement {
                    destination: shot.entry,
                    layer: 0,
                    sector: None,
                    gate_id: None,
                    line_id: None,
                    element: Some(entity_id),
                    flags: MoveFlags::empty(),
                    tolerance: 10.0,
                    direction: 0,
                    action: entry_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(ap);
                level += 1;
            }

            // -------- Jump gate branch --------
            //
            // After the approach/assert above, a jump gate emits a
            // single `Jump` generic element carrying the source and
            // destination jump-line indices.  The tick handler
            // consumes these in `start_jump`.
            if shot.is_jump {
                let (src, dst) = match (shot.jump_line_src, shot.jump_line_dst) {
                    (Some(s), Some(d)) => (s, d),
                    _ => {
                        tracing::warn!(
                            gate = %shot.door_index,
                            "Jump gate missing jump_line indices; skipping jump element"
                        );
                        prev_sector = Some(shot.new_sector);
                        continue;
                    }
                };
                let mut jump_elem =
                    SequenceElement::new_generic(level, Command::Jump, Some(entity_id));
                jump_elem.set_property(Field::JumplineSource, FieldValue::LineId(src));
                jump_elem.set_property(Field::JumplineDestination, FieldValue::LineId(dst));
                seq.append_element(jump_elem);
                level += 1;
                prev_sector = Some(shot.new_sector);
                continue;
            }

            // -------- Lockpick branch --------
            //
            // When the door is PC-locked and the PC has the lockpick
            // action, the sequence terminates after TURN + UNLOCK_DOOR
            // — the unlock animation flips `locked_pc` off and the
            // caller re-issues the move command to resume the path.
            if shot.is_locked_pc_unlockable && has_lockpick {
                // TURN toward the gate entry (point_in for direct,
                // point_out for indirect), so the sprite faces the
                // lock while picking it.
                let camera_pt = if shot.direct { shot.exit } else { shot.entry };
                let mut turn = SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                turn.set_property(
                    Field::CameraPoint,
                    FieldValue::GeoPoint2D {
                        x: camera_pt.x,
                        y: camera_pt.y,
                    },
                );
                seq.append_element(turn);
                level += 1;

                // UNLOCK_DOOR — the tick handler reads the door id
                // from `Field::Door` and picks UnlockingDoor vs
                // UnlockingTrap from the door table on its own.
                let mut unlock =
                    SequenceElement::new_generic(level, Command::UnlockDoor, Some(entity_id));
                unlock.set_property(Field::Door, FieldValue::DoorId(shot.door_index));
                seq.append_element(unlock);
                level += 1;

                // Early return — the lockpick animation will re-issue
                // the move once it terminates.
                ended_early = true;
                break;
            }

            // -------- Ladder lift wait --------
            if is_ladder_lift(self, shot.new_sector) {
                let mut wait = SequenceElement::new_movement(
                    level,
                    Command::WaitFreeLift,
                    Some(entity_id),
                    door_action,
                );
                wait.data = SequenceElementData::Movement {
                    destination: crate::coordinates::MapPoint::default(),
                    layer: 0,
                    sector: crate::position_interface::SectorHandle::new(shot.new_sector),
                    gate_id: Some(shot.door_index),
                    line_id: None,
                    element: None,
                    flags: MoveFlags::empty(),
                    tolerance: 0.0,
                    direction: 0,
                    action: door_action,
                    speed_factor,
                    post_seek_sequence: None,
                };
                seq.append_element(wait);
                level += 1;
            }

            // -------- PASS_DOOR + post-pass assert --------
            let mut pass = SequenceElement::new_movement(
                level,
                Command::PassDoor,
                Some(entity_id),
                door_action,
            );
            pass.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: shot.exit_layer,
                sector: None,
                gate_id: Some(shot.door_index),
                line_id: None,
                element: None,
                // Original PASS_DOOR constructor uses default flags
                // and only attaches the gate via SetGate.
                flags: MoveFlags::empty(),
                tolerance: 0.0,
                direction: 0,
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(pass);
            level += 1;

            // ASSERT_POSITION that the actor reached the exit point.
            let mut ap = SequenceElement::new_movement(
                level,
                Command::AssertPosition,
                Some(entity_id),
                door_action,
            );
            ap.data = SequenceElementData::Movement {
                destination: shot.exit,
                layer: 0,
                sector: None,
                gate_id: None,
                line_id: None,
                element: Some(entity_id),
                flags: MoveFlags::empty(),
                tolerance: 10.0,
                direction: 0,
                action: door_action,
                speed_factor,
                post_seek_sequence: None,
            };
            seq.append_element(ap);
            level += 1;

            prev_sector = Some(shot.new_sector);
        }

        // Clear TO_JUMP once we're past the last jump gate — the
        // trailing MOVE uses `initial_flags` unmodified.
        let trailing_flags = initial_flags;

        // Trailing emission.  Three goal shapes, three branches:
        //
        // * Point: emit MOVE to `goal_point`, subject to
        //   `move_after_last_door` and the building-sector
        //   short-circuit.
        // * Door: handle the goal-door's approach / CHANGE_POSITION
        //   into building / PC-lockpick tail.
        // * Line: emit MOVE with `MoveFlags::LINE` + `line_id` so
        //   arrival snaps to the line's tolerance window.
        if !ended_early {
            let last_into_building = prev_sector
                .map(|s| is_building_sector(self, s))
                .unwrap_or(false);

            match goal {
                GoalShape::Point(_) | GoalShape::Seek { .. } => {
                    if move_after_last_door && !last_into_building {
                        let (seek_target, seek_tolerance, seek_flags) = match goal {
                            GoalShape::Seek {
                                target, tolerance, ..
                            } => (Some(target), tolerance, trailing_flags | MoveFlags::SEEK),
                            _ => (None, 0.0, trailing_flags),
                        };
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // When SEEK is set and the last gate landed us
                    // inside a building sector, emit a trailing MOVE
                    // back to the last gate's `point_in` so the actor
                    // doesn't get stuck at the interior teleport point.
                    if last_into_building
                        && initial_flags.contains(MoveFlags::SEEK)
                        && let Some(last_shot) = gate_shots.last()
                    {
                        let (seek_target, seek_tolerance, seek_flags) = seek_goal
                            .map(|(target, tolerance)| {
                                (Some(target), tolerance, trailing_flags | MoveFlags::SEEK)
                            })
                            .unwrap_or((None, 0.0, trailing_flags));
                        let point_in = {
                            let host = self.mission_script.as_ref().and_then(|s| s.game_host());
                            host.and_then(|h| h.doors.get(usize::from(last_shot.door_index)))
                                .map(|d| MapPoint::new(d.point_in.0, d.point_in.1))
                                .unwrap_or(last_shot.exit)
                        };
                        let mut seek_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        seek_move.data = SequenceElementData::Movement {
                            destination: point_in,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: None,
                            element: seek_target,
                            flags: seek_flags,
                            tolerance: seek_tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(seek_move);
                    }
                }
                GoalShape::Line {
                    line_index,
                    tolerance,
                    ..
                } => {
                    // Emit `Move` to the line goal with
                    // `MoveFlags::LINE` and the line id.  When the
                    // last gate landed in a building, bail out without
                    // emitting.
                    if !last_into_building {
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: goal_point,
                            layer: goal_layer,
                            sector: None,
                            gate_id: None,
                            line_id: Some(line_index),
                            element: None,
                            flags: trailing_flags | MoveFlags::LINE,
                            tolerance,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                    }
                }
                GoalShape::Door {
                    door_index,
                    far_side_point,
                    far_side_layer,
                    far_side_is_building,
                } => {
                    // Hoist the goal door's PC-lockable lookup so the
                    // trailing lockpick tail fires regardless of which
                    // trailing branch was taken.  The lockpick tail is
                    // unconditional on the goal door's PC-locked flag,
                    // not on which branch (building vs non-building)
                    // was selected.
                    let goal_door_pc_lockable = {
                        let host = self.mission_script.as_ref().and_then(|s| s.game_host());
                        host.and_then(|h| h.doors.get(usize::from(door_index)))
                            .map(|d| d.locked_pc && d.unlockable)
                            .unwrap_or(false)
                    };
                    if !move_after_last_door {
                        // "Stop at the door" variant — caller set
                        // `move_after_last_door=false` to skip the
                        // trailing MOVE.  The gate-path includes the
                        // goal door as the last gate, so the loop
                        // already emitted approach + PASS_DOOR for it.
                        // Nothing to emit here.
                    } else if far_side_is_building {
                        // Random 0..30 frames wait + CHANGE_POSITION
                        // teleport into the building interior.  The
                        // direction stuffed on the element is the
                        // door's `point_out - point_in` sector-index.
                        let r: u32 = self.rng.u32(0..16) + self.rng.u32(0..16);
                        let mut wait = SequenceElement::new_generic(
                            level,
                            Command::WaitTimer,
                            Some(entity_id),
                        );
                        wait.set_property(Field::Timer, FieldValue::Integer(r));
                        seq.append_element(wait);
                        level += 1;

                        let (dx, dy) = {
                            let host = self.mission_script.as_ref().and_then(|s| s.game_host());
                            let d = host.and_then(|h| h.doors.get(usize::from(door_index)));
                            match d {
                                Some(d) => {
                                    (d.point_out.0 - d.point_in.0, d.point_out.1 - d.point_in.1)
                                }
                                None => (0.0, 0.0),
                            }
                        };
                        let dir = vector_to_sector_0_to_15(dx, dy);
                        let mut cp = SequenceElement::new_movement(
                            level,
                            Command::ChangePosition,
                            Some(entity_id),
                            base_action,
                        );
                        cp.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: prev_sector
                                .and_then(crate::position_interface::SectorHandle::new),
                            // No gate ref on the building-interior
                            // CHANGE_POSITION (it is an in-sector
                            // teleport).
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: dir,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(cp);
                        level += 1;
                    } else {
                        // Plain MOVE to the goal door's far-side
                        // point.  No `last_into_building` guard here —
                        // the trailing MOVE fires unconditionally for
                        // non-building goal doors.
                        let mut final_move = SequenceElement::new_movement(
                            level,
                            Command::Move,
                            Some(entity_id),
                            base_action,
                        );
                        final_move.data = SequenceElementData::Movement {
                            destination: far_side_point,
                            layer: far_side_layer,
                            sector: None,
                            // Original AppendMoveToDoorToSequence's
                            // trailing goal move is a plain MOVE to
                            // ptGoal; it does not call SetGate.
                            gate_id: None,
                            line_id: None,
                            element: None,
                            flags: trailing_flags,
                            tolerance: 0.0,
                            direction: 0,
                            action: base_action,
                            speed_factor,
                            post_seek_sequence: None,
                        };
                        seq.append_element(final_move);
                        level += 1;
                    }

                    // After the trailing MOVE / CHANGE_POSITION, if
                    // the goal door is PC-lockable and the actor has
                    // lockpick, emit TURN toward the lock +
                    // UNLOCK_DOOR.  The goal door is *not* included in
                    // `gate_path` for the door-goal case
                    // (`find_path_to_door` pops it), so the in-loop
                    // lockpick branch didn't fire for it — this is
                    // where the "walk up to door and pick it" finale
                    // is emitted.
                    if goal_door_pc_lockable && has_lockpick {
                        let (cam_pt, direct) = {
                            let host = self.mission_script.as_ref().and_then(|s| s.game_host());
                            let d = host.and_then(|h| h.doors.get(usize::from(door_index)));
                            // Use the path-direction the gate was
                            // approached in.  When the goal door was
                            // excluded from `gate_path` the caller
                            // signals that direction implicitly via
                            // `far_side_point` — it matches the
                            // door's near-side endpoint for the
                            // approach side.  Recover the direction
                            // by comparing endpoints.
                            let direct = d
                                .map(|d| {
                                    let dx = far_side_point.x - d.point_out.0;
                                    let dy = far_side_point.y - d.point_out.1;
                                    (dx * dx + dy * dy) < 1e-4
                                })
                                .unwrap_or(true);
                            let cam = d
                                .map(|d| {
                                    if direct {
                                        MapPoint::new(d.point_in.0, d.point_in.1)
                                    } else {
                                        MapPoint::new(d.point_out.0, d.point_out.1)
                                    }
                                })
                                .unwrap_or(far_side_point);
                            (cam, direct)
                        };
                        let _ = direct;
                        let mut turn =
                            SequenceElement::new_generic(level, Command::Turn, Some(entity_id));
                        turn.set_property(
                            Field::CameraPoint,
                            FieldValue::GeoPoint2D {
                                x: cam_pt.x,
                                y: cam_pt.y,
                            },
                        );
                        seq.append_element(turn);
                        level += 1;

                        let mut unlock = SequenceElement::new_generic(
                            level,
                            Command::UnlockDoor,
                            Some(entity_id),
                        );
                        unlock.set_property(Field::Door, FieldValue::DoorId(door_index));
                        seq.append_element(unlock);
                    }
                }
            }
        }

        for mut elem in tail_elements {
            elem.command_level = level;
            seq.append_element(elem);
            level += 1;
        }

        // Append a `SpeakHeroReachDestination` element at the tail of
        // the gate-movement sequence so the PC barks the "I have
        // arrived" line once the destination is reached.  Dispatched
        // at the same command_level as the last real element; the
        // PC's `Instruct` override terminates it on dispatch and
        // queues `HeroDoneCommand` via `arbitrate_instruct`.
        if append_arrival_speech && !seq.is_empty() {
            let speak_level = seq.last().map(|e| e.command_level).unwrap_or(level);
            let speak = SequenceElement::new(
                speak_level,
                Command::SpeakHeroReachDestination,
                Some(entity_id),
            );
            seq.append_element(speak);
        }

        // Append posture-recovery sub-elements right after the Speak
        // element so a PC mid-bow-aim / crouched / helping-climb /
        // simulating-beggar ends the order in a neutral posture
        // instead of frozen in their pre-move state.  Only fires for
        // PCs; `append_posture_recovery` bails on non-PC entities.
        if append_recovery {
            self.append_posture_recovery(entity_id, &mut seq);
        }

        let seq_id = self.launch_sequence(seq);
        tracing::trace!(
            entity = ?entity_id,
            ?seq_id,
            gates = gate_path.len(),
            early = ended_early,
            goal = ?goal,
            move_after_last_door,
            "Launched gate-traversal movement sequence"
        );

        // Destination markers are emitted by player group-move callers
        // only; AI/pathfinding callers use this helper without dropping
        // a ground mark.
    }

    /// Append posture-cleanup sub-elements at the tail of a PC move
    /// sequence so the PC ends the order in a neutral posture rather
    /// than frozen in the pre-move state.
    ///
    /// Covers:
    ///
    /// * **Shoot-bow drain** — if the sequence currently ends with a
    ///   `Command::ShootBow` element *and* the PC is no longer aiming,
    ///   demote that trailing element to `Command::ShootBowOnce` so the
    ///   queued shot fires exactly once before the walk resumes.
    /// * **Upright + bow-aim** → append `EQUIP_BOW` (re-arms the bow so
    ///   the aim state is re-entered after the walk).
    /// * **Crouched + last command ≠ CrouchUp** → append `CROUCH_DOWN`
    ///   so the PC re-crouches at the destination.
    /// * **HelpingToClimb** → append `ENTER_HELPING_CLIMB`.
    /// * **SimulatingBeggar** → append `ENTER_BEGGAR`.
    ///
    /// When the input sequence ends in SEEK, recovery is appended to
    /// that movement element's post-seek sub-sequence so it fires only
    /// on successful seek completion, not on seek abort.
    pub(crate) fn append_posture_recovery(
        &self,
        pc_id: EntityId,
        sequence: &mut crate::sequence::Sequence,
    ) {
        use crate::element::Command;
        let Some(entity) = self.get_entity(pc_id) else {
            return;
        };
        if !entity.is_pc() {
            return;
        }
        let posture = entity.element_data().posture;
        let Some(actor) = entity.actor_data() else {
            return;
        };
        let action_state = actor.action_state;

        // Drill into the SEEK element's post-seek sub-sequence when
        // the inbound sequence ends with `Command::Seek`; allocate a
        // fresh sub-sequence if none was attached yet, then append the
        // recovery command to it.  This way recovery fires only on
        // successful seek completion, not on seek abort.
        let target_sequence: &mut crate::sequence::Sequence = if sequence
            .last()
            .is_some_and(|last| last.command == Command::Seek)
        {
            let last_elem = sequence
                .elements
                .last_mut()
                .expect("Sequence::last() returned Some above");
            if let crate::sequence::SequenceElementData::Movement {
                post_seek_sequence, ..
            } = &mut last_elem.data
            {
                post_seek_sequence
                    .get_or_insert_with(|| Box::new(crate::sequence::Sequence::new()))
                    .as_mut()
            } else {
                sequence
            }
        } else {
            sequence
        };

        let (level, last_command) = match target_sequence.last() {
            None => (1u16, None),
            Some(last) => (last.command_level.saturating_add(1), Some(last.command)),
        };

        // "Shoot once then stop".
        if last_command == Some(Command::ShootBow) && !action_state.is_bow() {
            if let Some(last_mut) = target_sequence.elements.last_mut() {
                last_mut.command = Command::ShootBowOnce;
            }
            return;
        }

        match posture {
            crate::element::Posture::Upright if action_state.is_bow() => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EquipBow,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::Crouched if last_command != Some(Command::CrouchUp) => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::CrouchDown,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::HelpingToClimb => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterHelpingClimb,
                    Some(pc_id),
                ));
            }
            crate::element::Posture::SimulatingBeggar => {
                target_sequence.append_element(crate::sequence::SequenceElement::new(
                    level,
                    Command::EnterBeggar,
                    Some(pc_id),
                ));
            }
            _ => {}
        }
    }

    /// Enqueue an AI-initiated Move intent for this actor.
    ///
    /// Per-actor dedup: only one pending request per actor exists in
    /// the queue at any time — a later call for the same `entity_id`
    /// overwrites the earlier entry.  The actual Move element launch
    /// happens in `drain_pending_move_requests` at a deterministic
    /// point in the hourglass.
    ///
    /// This queue absorbs high-frequency AI re-fires (patrol macro-
    /// GoTo, pursuit re-pathfind) that would otherwise each spawn a
    /// fresh `Command::Move` element and `InterruptCurrent` the
    /// previous one at the same Normal priority, preventing the actor
    /// from ever completing a startup transition or making waypoint
    /// progress.
    ///
    /// Once drained, the Move element is launched via the sequence
    /// pipeline (`launch_element_for_owner` → `arbitrate_instruct` →
    /// `InstructOwner` dispatch), giving the move:
    /// * Priority arbitration — the element can be postponed behind
    ///   an in-flight `ENTER_ATTENTIVE_MODE`
    ///   (`PostponeEverythingButInjuries`) so the alerted-pose
    ///   transition finishes before the move starts.
    /// * System #16 — failed-path-impossible actually reaches the
    ///   owner via the Move's `element_impossible` condolation.
    /// * `post_process_path` on path arrival (see `tick.rs` Move
    ///   dispatch) inserts the startup-transition animation via the
    ///   normal pipeline.
    ///
    /// Run the AI `GoTo` pre-flight gates for an AI movement intent.
    /// Returns `true` if the intent should proceed to launch, `false`
    /// if it was rejected (in which case `couldnt_reachpoint` has been
    /// set on the AI controller and the caller should drop
    /// the intent).
    ///
    /// `intent.find_accessible` runs
    /// `FastFindGrid::find_authorized_position` against the actor's
    /// `MoveBox + (target_x, target_y)` and rewrites the intent target
    /// to the snapped centre on success.
    ///
    /// `intent.ask_obstacle` runs
    /// `FastFindGrid::is_straight_movement_authorized` from the
    /// actor's current position to the destination.  Only meaningful
    /// for straight moves (gated on `compute_direction == false`).
    fn preflight_ai_goto(
        &mut self,
        entity_id: EntityId,
        intent: &mut crate::order::AiOrderIntent,
    ) -> bool {
        // Upper-bound check.  `AiController::go_to` already rejects
        // `target_x <= 0 || target_y <= 0` before pushing the intent;
        // the engine drain owns the `>= GetLevelSize()` half because
        // `level_size` lives on the shared cutscene camera, not on
        // `AiContext`.
        let level_w = self.cutscene_camera.level_size.x;
        let level_h = self.cutscene_camera.level_size.y;
        if level_w > 0.0 && intent.target_x >= level_w
            || level_h > 0.0 && intent.target_y >= level_h
        {
            self.set_ai_couldnt_reachpoint(entity_id);
            return false;
        }

        if !intent.find_accessible && !intent.ask_obstacle {
            return true;
        }

        let (move_box, layer, position) = {
            let Some(entity) = self.get_entity(entity_id) else {
                return true;
            };
            let pi = entity.position_iface();
            let pm = pi.map_position();
            (
                *pi.get_move_box(),
                entity.element_data().layer(),
                pm.to_geo(),
            )
        };

        // Snap destination to the nearest authorised position when
        // `find_accessible` is set.  Translate the move box to the
        // requested destination and ask the grid.  On success rewrite
        // the intent target to the box centre.
        if intent.find_accessible {
            let dest = crate::geo2d::pt(intent.target_x, intent.target_y);
            let mut bbox = if move_box.is_somewhere() {
                crate::geo2d::BBox2D::from_corners(
                    crate::geo2d::pt(move_box.x_min() + dest.x, move_box.y_min() + dest.y),
                    crate::geo2d::pt(move_box.x_max() + dest.x, move_box.y_max() + dest.y),
                )
            } else {
                crate::geo2d::BBox2D::new()
            };
            if !self.fast_grid.find_authorized_position(&mut bbox, layer) {
                self.set_ai_couldnt_reachpoint(entity_id);
                return false;
            }
            let centre = bbox.center();
            intent.target_x = centre.x;
            intent.target_y = centre.y;
        }

        // Pre-flight straight movement.  Only meaningful for straight
        // moves (gated on `compute_direction == false`); when
        // `ask_obstacle` is set without straight-mode the check is
        // silently skipped rather than asserting.
        if intent.ask_obstacle && !intent.compute_direction {
            let dest = crate::geo2d::pt(intent.target_x, intent.target_y);
            if !self.fast_grid.is_straight_movement_authorized(
                position.into(),
                dest.into(),
                layer,
                &move_box,
            ) {
                self.set_ai_couldnt_reachpoint(entity_id);
                return false;
            }
        }

        true
    }

    /// Set `AiController::couldnt_reachpoint = true` on the entity, used
    /// by the GoTo pre-flight gates to surface a same-frame failure to
    /// the AI's stuck-retry / fallback logic.
    fn set_ai_couldnt_reachpoint(&mut self, entity_id: EntityId) {
        let Some(entity) = self
            .entities
            .get_mut(entity_id.0 as usize)
            .and_then(|e| e.as_mut())
        else {
            return;
        };
        if let Some(ai) = entity.ai_controller_mut() {
            ai.couldnt_reachpoint = true;
        }
    }

    fn launch_ai_move(&mut self, entity_id: EntityId, intent: &crate::order::AiOrderIntent) {
        // The AI think loop can legitimately emit two distinct `GoTo`
        // intents for the same actor in one tick (e.g. a SEEK retarget
        // dispatched immediately after a macro-GoTo).  `halt_actor`
        // tears down the prior Move element cleanly; here we just keep
        // the latest intent — "last intent wins", since the second
        // Halt invalidates anything the first request queued.
        let replaced = self
            .pending_move_requests
            .iter()
            .any(|(eid, _)| *eid == entity_id);
        if replaced {
            self.pending_move_requests
                .retain(|(eid, _)| *eid != entity_id);
            tracing::trace!(
                entity = ?entity_id,
                "launch_ai_move: replacing prior pending Move (AI re-issued GoTo this tick)"
            );
        }
        self.pending_move_requests.push((entity_id, intent.clone()));
    }

    /// Drain the pending-move-request queue and launch a Move
    /// sequence element for each.  Runs once per tick from the
    /// hourglass pipeline.  Determinism: requests drain in FIFO order
    /// of enqueue (a `Vec` with `retain`+`push` on launch preserves
    /// this).
    pub(super) fn drain_pending_move_requests(&mut self) {
        let requests = std::mem::take(&mut self.pending_move_requests);
        for (entity_id, intent) in requests {
            self.do_launch_ai_move(entity_id, &intent);
        }
    }

    /// Actually build and launch the Move sequence element for an AI
    /// intent.  Split out from `launch_ai_move` so the enqueue side
    /// can be cheap (push into a Vec) and the heavier work (resolve
    /// entity state, build element, run arbitration + path) only
    /// happens once per actor per tick at drain time.
    fn do_launch_ai_move(&mut self, entity_id: EntityId, intent: &crate::order::AiOrderIntent) {
        let dest = crate::coordinates::MapPoint {
            x: intent.target_x,
            y: intent.target_y,
        };
        let (layer, sector) = {
            let Some(entity) = self.get_entity(entity_id) else {
                tracing::warn!("do_launch_ai_move: entity {:?} not found", entity_id);
                return;
            };
            let ed = entity.element_data();
            (ed.layer(), ed.sector())
        };

        let action = intent.order_type;
        let mut elem = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(entity_id),
            action,
        );
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            layer: elem_layer,
            sector: elem_sector,
            flags,
            tolerance,
            element,
            speed_factor,
            ..
        } = &mut elem.data
        {
            *destination = dest;
            *elem_layer = layer;
            *elem_sector = sector;
            *flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(intent.move_flags));
            *tolerance = intent.tolerance;
            *element = intent.antagonist;
            *speed_factor = intent.speed_factor;
        }

        // Use the engine wrapper so posture stamping + synchronous
        // arbitrate_instruct + generate_transition fire on the
        // standard Instruct path.
        self.launch_element(elem);

        tracing::trace!(
            entity = ?entity_id,
            dest_x = dest.x,
            dest_y = dest.y,
            ?action,
            move_flags = intent.move_flags,
            "AI movement launched via sequence element"
        );
    }

    /// Issue a movement order for a specific entity to a map position.
    ///
    /// Routes through the sequence-element path: builds an
    /// `AiOrderIntent` on the fly and delegates to `launch_ai_move`,
    /// which launches a `Command::Move` element → `tick.rs`'s Move
    /// dispatch populates per-waypoint orders + runs
    /// `post_process_path`.  Used by PC right-click gate-fallback
    /// (movement.rs:788) when cross-sector routing has failed and we
    /// want a simple same-sector walk.
    pub(crate) fn issue_move_order(&mut self, entity_id: EntityId, dest: MapPoint, run: bool) {
        self.issue_move_order_ex(entity_id, dest, run, 0);
    }

    /// Issue a movement order with extra movement flags.
    ///
    /// `move_flags` carries `MoveFlags` bits (e.g. `RIDER_CHARGE`) from the
    /// AI decision through pathfinding to the actor's movement state.
    /// Routes through the sequence-element path by building an
    /// `AiOrderIntent` on the fly and delegating to `launch_ai_move`.
    pub(crate) fn issue_move_order_ex(
        &mut self,
        entity_id: EntityId,
        dest: MapPoint,
        run: bool,
        move_flags: u16,
    ) {
        // Check if entity is swordfighting — use sword walk animation.
        // Movement elements carry the animation from the caller;
        // swordfight seeks use the walking-with-sword non-animation.
        let is_swordfighting = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);
        let action = if is_swordfighting {
            if run {
                OrderType::RunningWithSword
            } else {
                OrderType::WalkingWithSword
            }
        } else if run {
            OrderType::RunningUpright
        } else {
            OrderType::WalkingUpright
        };

        let mut intent = crate::order::AiOrderIntent::new(action, dest.x, dest.y);
        intent.move_flags = move_flags;
        intent.no_halt = true; // callers handle halt themselves when they want it
        self.launch_ai_move(entity_id, &intent);
    }

    /// Advance all entities that have active paths (per-frame movement).
    ///
    /// For each entity with a non-empty `path_waypoints`, advances its
    /// position toward the current waypoint using [`movement::tick_movement`].
    /// When an entity arrives at its destination, either advances to the
    /// next [`ActiveDoorPass`] step or notifies the sequence manager that
    /// the movement element is terminated.
    ///
    /// Returns `(reached_entities, galopp_loop_entities)`.
    /// - `reached_entities`: entities that finished their path (need EventReachPoint).
    /// - `galopp_loop_entities`: rider entities that reached intermediate waypoints
    ///   with RIDER_CHARGE flag (need EventGaloppLoopEnd).
    pub(super) fn tick_entity_movement(
        &mut self,
        assets: &crate::engine::LevelAssets,
    ) -> (Vec<EntityId>, Vec<EntityId>) {
        if self.freeze_all {
            return (Vec::new(), Vec::new());
        }

        // Pre-pass: collect principal opponent positions for
        // combat-moving entities.  During sword/shield movement,
        // FaceOpponent / FaceDangerPoint overrides the entity's facing
        // direction toward their opponent instead of the movement
        // direction, and selects directional animations
        // (forward/backward/strafe) based on the angle between
        // movement and facing.
        let mut combat_face_targets: Vec<Option<crate::coordinates::MapPoint>> =
            vec![None; self.entities.len()];
        for (idx, slot) in self.entities.iter().enumerate() {
            let entity = match slot {
                Some(e) => e,
                None => continue,
            };
            let actor = match entity.actor_data() {
                Some(a) => a,
                None => continue,
            };
            let is_shield_moving = matches!(
                actor.action_state,
                crate::element::ActionState::MovingShield
            );
            // Shield bearers face the stored danger point.
            // Sword fighters face their principal opponent.
            if is_shield_moving && let Some(pt) = actor.shield_face_point {
                combat_face_targets[idx] = Some(pt);
                continue;
            }
            // Shield bearer with no danger point stored: face *away*
            // from the protected ally.  Encode this as a target equal
            // to `2 * self_pos - ally_pos` so the downstream
            // `vector_to_sector_0_to_15(target - self)` math aims the
            // shield-bearer away from the ally.
            if is_shield_moving
                && let Some(protected_id) = entity.pc_data().and_then(|pc| pc.shield_protected)
                && let Some(Some(ally)) = self.entities.get(protected_id.0 as usize)
            {
                let self_pos = entity.element_data().position_map();
                let ally_pos = ally.element_data().position_map();
                combat_face_targets[idx] = Some(crate::coordinates::MapPoint {
                    x: 2.0 * self_pos.x - ally_pos.x,
                    y: 2.0 * self_pos.y - ally_pos.y,
                });
                continue;
            }
            // FaceOpponent dispatch for sword movement:
            //   swordfighting → principal opponent's ground position
            //   else if soldier → primary target's ground position
            //   else            → return WALKING_SWORD without facing change
            //
            // Build this even before `action_state` flips to MovingSword;
            // forced sword movement can still be represented only by the
            // movement element's FORCE_SWORD_MOVEMENT flag at this point.
            //
            // The non-soldier, non-swordfighting branch leaves facing
            // untouched. We mirror this by storing the entity's own
            // position as a sentinel so the main loop's combat-face block
            // becomes a no-op (`fdx*fdx + fdy*fdy > 0.01` fails) while
            // still suppressing the movement-direction facing fallback.
            let is_swordfighting = entity
                .human_data()
                .map(|human| !human.opponents.is_empty())
                .unwrap_or(false);
            let opp_id_opt: Option<EntityId> = if is_swordfighting {
                // Principal opponent = first in opponent list.
                entity
                    .human_data()
                    .and_then(|h| h.opponents.first())
                    .copied()
            } else if entity.is_soldier() {
                // GetPrimaryTarget — soldier's AI-picked priority target,
                // which can differ from opponents[0].
                entity
                    .ai_controller()
                    .map(|c| EntityId(c.primary_target))
                    .filter(|id| id.0 != 0)
            } else {
                None
            };

            if let Some(opp_id) = opp_id_opt
                && let Some(Some(opp)) = self.entities.get(opp_id.0 as usize)
            {
                combat_face_targets[idx] = Some(opp.element_data().position_map());
            } else {
                // Sentinel: face self → no rotation, no movement-direction
                // fallback (the "return WALKING_SWORD without changing
                // facing" branch).
                combat_face_targets[idx] = Some(entity.element_data().position_map());
            }
        }

        // Pre-pass: look up the current sequence-element speed factor
        // for every entity with an active movement
        // (`distance *= speed_factor` during the per-frame motion
        // update). Pre-computed here so the main loop can borrow
        // `self.entities` mutably while consulting
        // `self.sequence_manager` for the factor.
        let mut speed_factors: Vec<f32> = vec![1.0; self.entities.len()];
        // Per-entity line-snap info: `(line_index, tolerance)` set
        // when the active movement element carries `MoveFlags::LINE`
        // and a `line_id`.  Drives the final-waypoint snap to the
        // nearest point on the line at arrival time, comparing the
        // line's distance against the element's tolerance.
        let mut line_snaps: Vec<Option<(crate::jump_line::JumpLineIndex, f32)>> =
            vec![None; self.entities.len()];
        // Per-entity final-waypoint tolerance snapshot for the arrival
        // check.  The seek-arrival predicate is:
        //
        //   target.sector == self.sector                     (same-sector)
        //   && dist_sq < seek_distance^2 * 1.1025            (5% margin)
        //
        // where `dist` is the vector from the actor to the target's
        // current position (or its current-row hotspot under
        // `USE_POINT`), with Y stretched by the inverse aspect ratio
        // when `DIRECTIONAL_TOLERANCE` is set.
        //
        // `target_is_actor` lets the main loop set the shield-follower
        // speed factor: when self is in `MovingShield` action state
        // and the seek target is an actor, the speed factor becomes
        // 1.0 / 1.5 / 2.0 depending on range.
        #[derive(Clone, Copy, Default)]
        struct FinalTol {
            tol: f32,
            directional: bool,
            target_sector: Option<crate::position_interface::SectorHandle>,
            target_is_actor: bool,
            /// Target's current `position_map`, sampled for this tick.
            /// The live seek target is checked every frame; the path
            /// waypoint can be stale after target movement or seek refresh.
            target_pos: Option<MapPoint>,
            /// Target's current-row hotspot offset for `USE_POINT` seeks.
            /// `None` when the flag is clear or the target's sprite has no
            /// per-row hotspot stored (falls back to plain position).
            use_point_offset: Option<MapVec>,
            /// Shield seeks compare actor position to the movement
            /// element's computed shield destination, not to the
            /// protected PC's live position.
            shield_destination: Option<MapPoint>,
            /// Snapshot of `ActorData::last_seek_target_position` —
            /// the target position stamped at seek launch / refresh.
            /// Used by the transition-animation refresh check to
            /// detect mid-walk target drift before the transition arm
            /// runs.
            last_seek_target_position: MapPoint,
            /// Whether the actor has a `post_seek_sequence` attached.
            /// Lifts the `is_final_waypoint` gate on tolerance arrival
            /// for mid-path arrivals: the seek's same-sector +
            /// tolerance predicate runs every tick, not just at the
            /// final waypoint.  When the target wanders into range
            /// mid-route, the seek terminates early and the post-seek
            /// sequence fires.  Without a post-seek sequence to
            /// consume the arrival, the order_pop fall-through would
            /// drop intermediate waypoints and leave the actor
            /// stranded — so guard intermediate-tick arrival on this
            /// flag.
            has_post_seek: bool,
        }
        let mut final_tolerances: Vec<FinalTol> = vec![FinalTol::default(); self.entities.len()];
        let mut point_seek_post_sectors: Vec<Option<crate::position_interface::SectorHandle>> =
            vec![None; self.entities.len()];
        let mut sword_movement_starts: Vec<EntityId> = Vec::new();
        let mut sword_movement_terminations: Vec<EntityId> = Vec::new();
        for (idx, slot) in self.entities.iter().enumerate() {
            let Some(entity) = slot else { continue };
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let entity_id = EntityId(idx as u32);
            let Some((seq_id, elem_idx)) = self
                .sequence_manager
                .in_progress_element_for_actor_matching(entity_id, |e| e.data.is_movement())
            else {
                continue;
            };
            if let Some(elem) = self.sequence_manager.get_element(seq_id, elem_idx) {
                speed_factors[idx] = elem.speed_factor();
                if let crate::sequence::SequenceElementData::Movement {
                    flags,
                    line_id,
                    tolerance,
                    element: target_elem,
                    destination,
                    sector,
                    ..
                } = &elem.data
                {
                    if line_id.is_some() && flags.contains(crate::sequence::MoveFlags::LINE) {
                        line_snaps[idx] = Some((line_id.unwrap(), *tolerance));
                    } else if *tolerance > 0.0 && flags.contains(crate::sequence::MoveFlags::SEEK) {
                        // The per-tick seek-arrival predicate (and its
                        // FROZEN-wait sibling) is a SEEK-only
                        // mechanism.  Non-seek `GoNear`-style
                        // stop-distances are enforced earlier, by
                        // `insert_transition_end` adding the element's
                        // tolerance to the end-transition shift
                        // (`distance_remaining + tolerance`), which
                        // truncates the walking phase before order
                        // emission.  Gating here keeps the FinalTol
                        // snapshot meaningful only for true seeks, so
                        // the downstream tolerance-arrival check can
                        // rely on `target_pos` / `shield_destination`
                        // being live.
                        let directional =
                            flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE);
                        let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
                        let seek_shield = flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD);
                        let (target_sector, target_is_actor, target_pos, use_point_offset) =
                            match target_elem.and_then(|id| self.get_entity(id)) {
                                Some(t) => {
                                    let target_elem_data = t.element_data();
                                    let target_pos = target_elem_data.position_map();
                                    // Fall back to plain position when
                                    // the hotspot is zero (no per-row
                                    // point information).
                                    let offset = if use_point {
                                        target_elem_data
                                            .sprite
                                            .current_hotspot()
                                            .filter(|p| p.x != 0.0 || p.y != 0.0)
                                            .map(|p| MapVec::new(p.x, p.y))
                                    } else {
                                        None
                                    };
                                    (
                                        target_elem_data.sector(),
                                        t.actor_data().is_some(),
                                        Some(target_pos),
                                        offset,
                                    )
                                }
                                // SEEK without antagonist = seek-to-point
                                // mode.  Skip the dist-vs-tolerance
                                // check; arrival is detected by motion
                                // termination + same-sector match.
                                // Falls through to the standard
                                // `dist <= speed` final-waypoint
                                // arrival when there is no post-seek
                                // sequence.  Leaving target_pos None
                                // signals the consumer to skip the
                                // entity-target seek-distance check.
                                None => {
                                    if actor.post_seek_sequence.is_some() {
                                        point_seek_post_sectors[idx] = *sector;
                                    }
                                    (None, false, None, None)
                                }
                            };
                        // Skip the FinalTol snapshot entirely for
                        // seek-to-point + non-shield (target_pos is
                        // None and there's no shield destination), so
                        // the seek-arrival predicate doesn't fire.
                        if target_pos.is_some() || seek_shield {
                            final_tolerances[idx] = FinalTol {
                                tol: *tolerance,
                                directional,
                                target_sector,
                                target_is_actor,
                                target_pos,
                                use_point_offset,
                                shield_destination: seek_shield.then_some(*destination),
                                last_seek_target_position: actor.last_seek_target_position,
                                has_post_seek: actor.post_seek_sequence.is_some(),
                            };
                        }
                    }
                }
            }
        }

        // Pre-pass: drive the per-tick `TurnDrunken()` turn for every
        // drunken soldier and record the resulting sprite facing as
        // an override for the main loop.  `TurnDrunken()` picks
        // between `TurnSlow(2)` and `TurnVerySlow()` (delay 5) so the
        // soldier's facing lags behind the movement vector.  This
        // must run before the main loop because the per-tick turn
        // advances `position_iface` (a mutable borrow that would
        // conflict with `entity.element_data_mut()`).
        let mut drunk_turn_overrides: Vec<Option<i16>> = vec![None; self.entities.len()];
        for (idx, slot) in self.entities.iter_mut().enumerate() {
            let Some(entity) = slot else { continue };
            if !matches!(entity, crate::element::Entity::Soldier(_)) {
                continue;
            }
            let is_drunk = entity
                .npc_data()
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.blood_alcohol > 0)
                .unwrap_or(false);
            if !is_drunk {
                continue;
            }
            // Compute the movement goal vector.  Skip entities without
            // an active movement path — idle drunk soldiers don't
            // wobble.  Goal is read from the actor's Move element's
            // current order (authoritative path source).
            let entity_id = EntityId(idx as u32);
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let Some(_) = actor.active_movement.sequence_id else {
                continue;
            };
            let Some((_, _, order)) = self.sequence_manager.current_order_for_actor(entity_id)
            else {
                continue;
            };
            let goal = crate::geo2d::pt(order.target_x, order.target_y);
            let pos = entity.element_data().position_map();
            let dx = goal.x - pos.x;
            let dy = goal.y - pos.y;
            if dx * dx + dy * dy < 0.01 {
                continue;
            }
            let goal_sector = vector_to_sector_0_to_15(dx, dy);
            // Gate the facing-from-movement-vector goal update on
            // the order's compute_direction flag.  When the order
            // pushes a fixed facing (compute_direction = false), keep
            // the goal direction the caller set and only run the slow
            // turn — `TurnDrunken` reads the direction goal but never
            // writes it.
            let order_compute_direction = order.compute_direction;
            if entity.actor_data().is_some() {
                let pi = entity.position_iface_mut();
                let current_dir = pi.get_direction();
                let goal_for_turn = if order_compute_direction {
                    pi.set_direction(crate::position_interface::Direction::from_raw(
                        goal_sector as i32,
                    ));
                    goal_sector as u16
                } else {
                    u16::from(pi.get_direction_goal())
                };
                let very_slow = crate::engine::soldier_helpers::turn_drunken_is_very_slow(
                    u16::from(current_dir),
                    goal_for_turn,
                );
                if very_slow {
                    pi.turn_very_slow();
                } else {
                    pi.turn_slow(2);
                }
                drunk_turn_overrides[idx] = Some(i16::from(pi.get_direction()));
            }
        }

        // Pre-pass: per-entity current-sector lift translation, for
        // the lift branches of the movement-animation derivation.
        // When a moving actor is in a lift sector, the per-frame
        // walk/run animation is overridden by the lift's upwards /
        // downwards action mapping:
        //   * Upright posture: lift type rewrites the action; upwards
        //     and downwards animations are equal for upright, so we
        //     always use the upwards mapping.
        //   * OnLadder / OnWall: pick upwards vs downwards by
        //     dot-producting the ladder vector (`pt_low - pt_high`)
        //     with the movement vector — non-negative means moving
        //     down.  The high / low exit points are the in-side
        //     points of the lift's highest and lowest doors.
        //
        // Pre-computed here so the main loop can borrow `self.entities`
        // mutably without touching `self.fast_grid` or the door table.
        let mut lift_translations: Vec<Option<LiftAnimContext>> = vec![None; self.entities.len()];
        let mut door_pass_wall_directions: Vec<Option<i16>> = vec![None; self.entities.len()];
        for (idx, slot) in self.entities.iter().enumerate() {
            let Some(entity) = slot else { continue };
            let posture = entity.element_data().posture;
            let door_pass_action = entity
                .actor_data()
                .and_then(|actor| actor.active_door_pass.as_ref())
                .map(|dp| dp.current_action);
            let Some(sector) = entity.element_data().sector() else {
                continue;
            };
            let Some(gs) =
                self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector)))
            else {
                continue;
            };
            if matches!(
                door_pass_action,
                Some(
                    OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                )
            ) {
                door_pass_wall_directions[idx] = entity
                    .actor_data()
                    .and_then(|actor| actor.active_door_pass.as_ref())
                    .and_then(|dp| {
                        self.mission_script
                            .as_ref()
                            .and_then(|s| s.game_host())
                            .and_then(|host| host.doors.get(usize::from(dp.door_index)))
                            .map(|door| door.sector_in)
                    })
                    .and_then(|sector_in| {
                        self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(
                            sector_in,
                        )))
                    })
                    .and_then(|sector| {
                        if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                            Some(sector.lift_direction)
                        } else {
                            None
                        }
                    });
            }
            let Some(lt) = gs.lift_type else { continue };
            match posture {
                crate::element::Posture::Upright => {
                    lift_translations[idx] = Some(LiftAnimContext::Upright(lt));
                }
                crate::element::Posture::OnLadder | crate::element::Posture::OnWall
                    if matches!(
                        (posture, lt, door_pass_action),
                        (
                            crate::element::Posture::OnWall,
                            crate::sector::LiftType::Wall,
                            _
                        ) | (
                            crate::element::Posture::OnLadder,
                            crate::sector::LiftType::Ladder,
                            _
                        )
                    ) =>
                {
                    let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
                    let ladder_dx = pt_low.x - pt_high.x;
                    let ladder_dy = pt_low.y - pt_high.y;
                    lift_translations[idx] = Some(LiftAnimContext::OnClimb {
                        lift_type: lt,
                        lift_direction: gs.lift_direction,
                        ladder_dx,
                        ladder_dy,
                    });
                }
                _ => {}
            }
            if lift_translations[idx].is_none()
                && matches!(
                    (lt, door_pass_action),
                    (
                        crate::sector::LiftType::Wall,
                        Some(
                            OrderType::ClimbingWallUp
                                | OrderType::ClimbingWallDown
                                | OrderType::ClimbingWallUpFast
                                | OrderType::ClimbingWallDownFast
                        )
                    ) | (
                        crate::sector::LiftType::Ladder,
                        Some(
                            OrderType::ClimbingLadderUp
                                | OrderType::ClimbingLadderDown
                                | OrderType::ClimbingLadderUpFast
                                | OrderType::ClimbingLadderDownFast
                        )
                    )
                )
            {
                let (pt_low, pt_high) = self.lift_endpoint_points(gs.sector_number);
                lift_translations[idx] = Some(LiftAnimContext::OnClimb {
                    lift_type: lt,
                    lift_direction: gs.lift_direction,
                    ladder_dx: pt_low.x - pt_high.x,
                    ladder_dy: pt_low.y - pt_high.y,
                });
            }
        }

        // Pre-pass: snapshot every actor's position / layer / sector /
        // posture / repulsive-point contribution for the
        // anti-collision disturbing-actor lookup.  Captured once per
        // tick so the mutable main loop can read neighbour state
        // without a second borrow, matching the deterministic
        // start-of-tick view the replay system relies on.  No
        // mobile-element branch — none ship in this game.
        // Mutable — each entity's post-move position is written back
        // so later entities in the same tick see the serial
        // "already-moved" view: each actor's anti-collision lookup
        // reads live positions from earlier-processed actors.
        let mut anti_snapshots: Vec<Option<super::anti_collision::ActorSnapshot>> =
            super::anti_collision::snapshot_all(
                &self.entities,
                &self.sequence_manager,
                &assets.profile_manager,
            );

        // Collect movement results that need sequence manager notification.
        // We can't call sequence_manager while iterating entities mutably.
        let mut arrived: Vec<(EntityId, ActiveMovement)> = Vec::new();
        // Door-pass triggers to execute after the movement loop (need &mut self).
        let mut door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)> = Vec::new();
        // Door-pass Transition orders to push onto the actor's current
        // sequence element after the loop closes (needs sequence_manager).
        let mut transition_pushes: Vec<(crate::sequence::SequenceId, usize, crate::order::Order)> =
            Vec::new();
        // Pending `DoorPassStep::Select` hulk requests — processed after the
        // loop since they mutate both the carrier and its carried target.
        let mut select_triggers: Vec<(EntityId, f32)> = Vec::new();
        let mut completed_door_passes: Vec<(EntityId, crate::gate::DoorIndex, bool)> = Vec::new();
        // Rider entities whose running animation hit the charge
        // decision frames while carrying RIDER_CHARGE.
        let mut galopp_events: Vec<EntityId> = Vec::new();
        // Movement elements whose sprite motion returned the blocked-
        // abort signal and must be marked Impossible after the entity
        // borrow ends.
        let mut blocked_impossible: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();
        let mut door_pass_transition_done_effects: Vec<EntityId> = Vec::new();
        let mut post_seek_arrivals: Vec<(EntityId, crate::sequence::SequenceId, usize)> =
            Vec::new();
        // Elevation-line crossings detected during this tick. Dispatched
        // after the entity loop so `check_for_line_crossing` can borrow
        // `self` for the fast-grid query and obstacle swap.
        // Each entry is `(entity_id, old_pos, new_pos, layer)` in projected
        // map coordinates. Geometry queries convert at the call boundary.
        let mut line_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Patch-line (`LINE_PATCH`) crossings detected for PC actors.
        // Dispatched after the entity loop so
        // `check_for_patch_line_crossing` can borrow `self` for the
        // per-patch `enter`/`leave`/`apply` state mutation and effect
        // processing.  Covers the `LINE_PATCH` PC-only crossing arm.
        let mut patch_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Sound-line (`LINE_SOUND`) crossings detected for any actor.
        // Dispatched after the entity loop so
        // `check_for_sound_line_crossing` can borrow `self` for the
        // material refresh.  Covers the `LINE_SOUND` crossing arm
        // (single-line and multi-line).  Unlike LINE_PATCH this fires
        // for every actor (PC, NPC, soldier) — the SOUND arm is not
        // gated on PC.
        let mut sound_cross_checks: Vec<(EntityId, MapPoint, MapPoint, u16)> = Vec::new();
        // Seek elements whose end-of-walk arrival put a
        // transition-to-waiting animation in line as the next order
        // while the live target had drifted beyond
        // `transition_distance + seek_distance × 1.05`.  Seek refresh
        // aborts the queued transition arm and rebuilds the path; the
        // Rust port queues these for the existing `tick_refresh_seeks`
        // machinery to handle right after the per-tick movement loop,
        // before the sequence manager re-dispatches.  Each entry is
        // `(owner, seq_id, elem_idx)`.
        let mut transition_seek_refreshes: Vec<(EntityId, crate::sequence::SequenceId, usize)> =
            Vec::new();
        // Waypoint arrivals (both intermediate and final) — each
        // triggers one `do_next_order` call on the actor's Move
        // element to pop the walking order that represented that
        // waypoint.  Each waypoint is its own order on the actor's
        // movement order list, and the engine pops them as the actor
        // crosses them.  Collected here and processed after the entity
        // loop so the `do_next_order` call can borrow `self` mutably.
        let mut order_pops: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();

        // Water-splash titbit emissions queued from the walk branch.
        // Drained after the entity loop so `titbit_manager.add_titbit`
        // can borrow `&mut self` without colliding with the active
        // entity borrow.
        let mut water_splash_emits: Vec<(EntityId, crate::coordinates::WorldPoint3D, u16)> =
            Vec::new();
        let mut movement_state_effects: Vec<(
            EntityId,
            crate::element::Posture,
            crate::element::ActionState,
        )> = Vec::new();

        for (idx, slot) in self.entities.iter_mut().enumerate() {
            let entity = match slot {
                Some(e) => e,
                None => continue,
            };
            // Check swordfight status before mutable borrows — needed at
            // movement completion to preserve WaitingSword (idle state
            // is derived from the action state machine, not hardcoded
            // Waiting).
            let is_swordfighting = entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);

            // Extract movement data from actor (scoped borrow).
            //
            // The walk goal is read from the current order's
            // destination on the actor's active Move element —
            // accessed via `SequenceManager::current_order_for_actor`.
            // `path_waypoints` is kept as a mirror for legacy bolt-ons
            // (drunken wobble, abilities, debug overlays) but is no
            // longer the authoritative path source in the hot loop.
            let (
                goal,
                action_state,
                order_id,
                door_pass_anim,
                is_final_waypoint,
                order_action,
                move_seq_id,
                move_elem_idx,
                active_move_flags,
                order_tolerance,
                order_compute_direction,
                order_reverse,
                next_destination_same_action,
            ) = {
                let actor = match entity.actor_data_mut() {
                    Some(a) => a,
                    None => continue,
                };
                if !actor.action_state.is_moving()
                    && actor.action_state != crate::element::ActionState::MovingSword
                    && actor.action_state != crate::element::ActionState::MovingFastSword
                    && actor.action_state != crate::element::ActionState::MovingShield
                {
                    continue;
                }
                let entity_id_inner = EntityId(idx as u32);
                // Read goal from the current **movement** element's
                // front order on the Move / PassDoor / Seek element.
                //
                // We explicitly filter by element data type instead
                // of using `current_order_for_actor` directly: another
                // element type (`Turn`, `Generic` animation, …) may
                // have become InProgress concurrently — e.g. a Turn
                // launched at `SequencePriority::Turn` while the Move
                // is still in flight.  Its front order has no
                // destination (`Turning` orders are (0,0)), so using
                // it as a goal would make the actor walk toward the
                // map origin.  Hold a pointer to the *movement*
                // element specifically by picking the InProgress
                // element whose data is a `Movement`.
                let move_elem = self
                    .sequence_manager
                    .in_progress_element_for_actor_matching(entity_id_inner, |e| {
                        e.data.is_movement()
                    });
                let Some((seq_id, elem_idx)) = move_elem else {
                    // No active Move element (element terminated or
                    // was never active) — drop out of the moving
                    // state back to Waiting.
                    let restore_anti_collision = {
                        let restore_anti_collision = actor.active_door_pass.is_some();
                        if restore_anti_collision {
                            tracing::warn!(
                                entity = ?entity_id_inner,
                                "DoorPass: clearing stale active pass after movement element disappeared"
                            );
                            actor.active_door_pass = None;
                        }
                        actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                            crate::element::ActionState::WaitingSword
                        } else {
                            crate::element::ActionState::Waiting
                        };
                        actor.active_movement.clear();
                        restore_anti_collision
                    };
                    if restore_anti_collision {
                        entity.position_iface_mut().set_anti_collision_on(true);
                    }
                    continue;
                };
                let Some(order) = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.current_order())
                else {
                    continue;
                };
                let goal = MapPoint::new(order.target_x, order.target_y);
                let order_id = Some(order.order_id);
                let order_action = order.order_type;
                let order_tolerance = order.tolerance;
                let order_compute_direction = order.compute_direction;
                let order_reverse = order.reverse;
                let next_destination_same_action = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.next_order())
                    .filter(|next| next.order_type == order_action)
                    .map(|next| MapPoint::new(next.target_x, next.target_y));
                let active_move_flags = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| match &e.data {
                        crate::sequence::SequenceElementData::Movement { flags, .. } => {
                            Some(*flags)
                        }
                        _ => None,
                    })
                    .unwrap_or(crate::sequence::MoveFlags::empty());

                // Is this the literal last order in the queue?  The
                // Movement element's `tolerance` applies to the final
                // arrival (tolerance applies only on the last order),
                // so we must only allow `tolerance_arrival`
                // to short-circuit when *no* orders remain behind the
                // current one — including end-transition orders spliced
                // in by `insert_transition_end`, which still carry the
                // actual destination as their target.  A prior version
                // of this check counted "last walk-style order", which
                // made the penultimate walking order inserted by
                // `insert_transition_end` look final and triggered an
                // instant tolerance arrival the moment the start
                // transition popped — the actor teleported past the
                // walking phase, played the stop transition in place
                // and never covered any ground.
                let is_final_waypoint = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|e| e.orders.len() <= 1)
                    .unwrap_or(true);

                // Use the animation from the active door-pass Walk step.
                let door_pass_anim: Option<OrderType> =
                    actor.active_door_pass.as_ref().map(|dp| dp.current_action);

                (
                    goal,
                    actor.action_state,
                    order_id,
                    door_pass_anim,
                    is_final_waypoint,
                    order_action,
                    seq_id,
                    elem_idx,
                    active_move_flags,
                    order_tolerance,
                    order_compute_direction,
                    order_reverse,
                    next_destination_same_action,
                )
            };

            let elem = entity.element_data_mut();
            let dx = goal.x - elem.position_map().x;
            let dy = goal.y - elem.position_map().y;
            let dist = (dx * dx + dy * dy).sqrt();
            // Combat movement: face opponent, select directional
            // animation.  `compute_direction=false` (don't auto-face
            // movement direction), face toward opponent, pick
            // forward/backward/strafe animation based on angle between
            // movement vector and facing vector.
            let combat_target = combat_face_targets[idx];
            let door_pass_sword_nonanimation =
                door_pass_anim.is_some_and(is_sword_movement_nonanimation);
            let order_sword_nonanimation = is_sword_movement_nonanimation(order_action);
            let forced_sword_motion = active_move_flags.intersects(
                crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT
                    | crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT,
            );
            let is_sword_motion = matches!(
                action_state,
                crate::element::ActionState::MovingSword
                    | crate::element::ActionState::MovingFastSword
            ) || door_pass_sword_nonanimation
                || order_sword_nonanimation
                || forced_sword_motion;
            let is_shield_motion =
                matches!(action_state, crate::element::ActionState::MovingShield);
            let is_combat = (is_shield_motion && combat_target.is_some()) || is_sword_motion;
            if is_combat {
                // Face opponent instead of movement direction.  Use
                // `set_direction_goal` + per-frame `turn()` rather
                // than instantly snapping facing, so the facing
                // rotates one step per frame toward the opponent.
                if let Some(opp_pos) = combat_target {
                    let fdx = opp_pos.x - elem.position_map().x;
                    let fdy = opp_pos.y - elem.position_map().y;
                    if fdx * fdx + fdy * fdy > 0.01 {
                        elem.set_direction_goal(vector_to_sector_0_to_15(fdx, fdy));
                    }
                }
            } else if dist > 0.01 && order_compute_direction {
                // Normal movement: face movement direction.  Use
                // `set_direction_goal` (not instant) so the per-frame
                // turn rotates one sector per tick toward the movement
                // vector.  Paired with the 0.6× turn-slowdown in
                // `perform_motion` below, this reproduces the
                // "character pivots before walking" feel from the
                // original game.
                //
                // Gated on `order.compute_direction`: facing is only
                // updated when the order's `compute_direction` flag is
                // true.  Orders pushed by AI for in-place transitions
                // / posture changes / TurnDrunken-driven walks set it
                // false so the actor's facing is preserved by the
                // caller.
                //
                // `order.reverse` flips the facing 180° (sector ^ 8)
                // so reverse-walked animations face away from the
                // movement vector.
                let raw =
                    drunk_turn_overrides[idx].unwrap_or_else(|| vector_to_sector_0_to_15(dx, dy));
                let dir = if order_reverse { raw ^ 8 } else { raw };
                elem.set_direction_goal(dir);
            }

            // Choose animation based on action state and movement angle.
            let anim = if let Some(dp_anim) =
                door_pass_anim.filter(|anim| !is_sword_movement_nonanimation(*anim))
            {
                dp_anim
            } else if is_combat {
                if is_sword_motion && combat_target.is_none() {
                    // Plain WALKING_SWORD when a non-soldier is forced
                    // through sword movement without an active
                    // opponent.  The `WalkingWithSword` /
                    // `RunningWithSword` values are non-animations and
                    // must never be sent directly to the per-frame
                    // motion update.
                    OrderType::WalkingSword
                } else {
                    // Compute angle between movement direction and
                    // facing direction, normalised to [0, 2π).
                    // UNIT = π/4 (45°).  8-sector mapping:
                    //   [0, π/4) or [7π/4, 2π) → forward
                    //   [π/4, 3π/4)             → strafe right
                    //   [3π/4, 5π/4)            → backward
                    //   [5π/4, 7π/4)            → strafe left
                    let facing_angle = if let Some(opp_pos) = combat_target {
                        let fdx = opp_pos.x - elem.position_map().x;
                        let fdy = opp_pos.y - elem.position_map().y;
                        if fdx * fdx + fdy * fdy > 0.01 {
                            fdy.atan2(fdx)
                        } else {
                            (elem.direction() as f32) * std::f32::consts::PI / 8.0
                        }
                    } else {
                        (elem.direction() as f32) * std::f32::consts::PI / 8.0
                    };
                    let move_angle = dy.atan2(dx);
                    // Angle from facing to movement, normalised to [0, 2π).
                    let mut angle = move_angle - facing_angle;
                    if angle < 0.0 {
                        angle += 2.0 * std::f32::consts::PI;
                    }
                    if angle >= 2.0 * std::f32::consts::PI {
                        angle -= 2.0 * std::f32::consts::PI;
                    }

                    let unit = std::f32::consts::FRAC_PI_4; // π/4 = 45°
                    match action_state {
                        crate::element::ActionState::MovingShield => {
                            if angle < unit || angle >= 7.0 * unit {
                                OrderType::WalkingShield
                            } else if angle < 3.0 * unit {
                                OrderType::StrafingRightShield
                            } else if angle < 5.0 * unit {
                                OrderType::WalkingBackwardsShield
                            } else {
                                OrderType::StrafingLeftShield
                            }
                        }
                        // MovingSword and MovingFastSword both use the
                        // directional walking/strafing sword
                        // animations — the `fast` flag is ignored when
                        // selecting the animation.  Running in combat
                        // is implemented by playing the walking anim
                        // under `MotionMethod::Fast` (2× frame rate +
                        // 2× distance).
                        _ => {
                            if angle < unit || angle >= 7.0 * unit {
                                OrderType::WalkingSword
                            } else if angle < 3.0 * unit {
                                OrderType::StrafingRightSword
                            } else if angle < 5.0 * unit {
                                OrderType::WalkingBackwardsSword
                            } else {
                                OrderType::StrafingLeftSword
                            }
                        }
                    }
                }
            } else {
                // Animation comes from the current order's type —
                // dispatch is on `order.action`.  Order types get
                // rewritten by `MakeFast` / `MakeSlow` / `MakeUpright`
                // / `MakeCrouched`, so reading the order directly is
                // how a mid-movement speed change propagates to the
                // sprite.  Falls back to an action_state-derived base
                // only when the order type isn't a movement animation
                // (shouldn't happen for a Move element but is
                // defensive).
                let base = match order_action {
                    OrderType::WalkingUpright
                    | OrderType::WalkingCrouched
                    | OrderType::WalkingAlerted
                    | OrderType::RunningUpright
                    | OrderType::TransitionWalkingUprightRunningUpright
                    | OrderType::TransitionRunningUprightWalkingUpright
                    | OrderType::TransitionWaitingUprightWalkingUpright
                    | OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionWaitingUprightRunningUpright
                    | OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWalkingUpright
                    | OrderType::TransitionWalkingUprightWalkingCrouched
                    | OrderType::TransitionWalkingCrouchedRunningUpright
                    | OrderType::TransitionRunningUprightWalkingCrouched
                    | OrderType::TransitionWaitingCrouchedWalkingCrouched
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
                    | OrderType::TransitionWaitingUprightSpecial
                    | OrderType::TransitionSpecialWaitingUpright
                    | OrderType::TransitionWaitingUprightBoredWaitingUpright
                    | OrderType::TransitionWaitingUprightWaitingUprightBored
                    | OrderType::TransitionCrouchingUp
                    | OrderType::TransitionCrouchingDown
                    | OrderType::TransitionSittingWaitingUpright
                    | OrderType::TransitionLeaningOutWaitingAlerted
                    | OrderType::LoweringShield
                    | OrderType::WalkingStairs
                    | OrderType::RunningStairs
                    | OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast
                    | OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast => order_action,
                    _ => match action_state {
                        crate::element::ActionState::MovingFast => OrderType::RunningUpright,
                        _ => OrderType::WalkingUpright,
                    },
                };
                // Lift branches: when a moving actor is in a lift
                // sector, the lift type rewrites the per-frame
                // animation.  Upright posture takes the upwards
                // mapping unconditionally; on-ladder / on-wall posture
                // chooses upwards vs downwards by dot-producting the
                // ladder vector (`pt_low - pt_high`) with the movement
                // vector — non-negative means moving down.  Snapshotted
                // in `lift_translations` so we don't have to re-borrow
                // `self.fast_grid` or the door table mid-loop.
                match lift_translations[idx] {
                    Some(LiftAnimContext::Upright(lt)) => lt.translate_upright_action(base),
                    Some(LiftAnimContext::OnClimb {
                        lift_type,
                        lift_direction: _,
                        ladder_dx,
                        ladder_dy,
                    }) => {
                        let going_down = ladder_dx * dx + ladder_dy * dy >= 0.0;
                        lift_type.translate_climb_action(base, going_down)
                    }
                    None => base,
                }
            };
            // Advance sprite animation and get per-frame distance.
            // PerformMotion sets `row = conversion[anim] + direction`,
            // increments the frame, then reads `GetDistance(row,
            // frame)` only when `frame_count == 0` (the first tick of
            // a new animation frame).  Between frames the distance is
            // 0, so entities move in discrete steps synced to the
            // animation.
            //
            // Motion methods:
            //   Walk / Run: normal frame distance * speed_factor
            //   Fast: double frame rate + double distance (only used
            //     for RUNNING_WITH_SWORD in combat, NOT for normal
            //     running)
            // Normal running uses Run, which is identical to Walk in
            // distance calculation — only the animation differs.  The
            // running animation's per-frame distances in the sprite
            // data are already larger than walking distances.
            //
            // The per-frame sprite distance is scaled by the active
            // sequence element's speed factor.  PC-issued moves use
            // 1.0; shield-following and the AI patrol/approach paths
            // set variable factors.
            //
            // Shield-follower speed adjust: when a PC in MovingShield
            // action state is seeking an actor target (the shield
            // holder), the sequence element's speed factor is
            // rewritten per tick to close gaps quickly and slow down
            // when near.
            //   dist² < 25  → 1.0
            //   dist² < 100 → 1.5
            //   else        → 2.0
            // We override the captured value so `current_frame_distance
            // * speed_factor` sees the adjusted value this tick.  The
            // captured value is reread from the element next tick.
            {
                let ft = final_tolerances[idx];
                if ft.tol > 0.0
                    && ft.target_is_actor
                    && matches!(action_state, crate::element::ActionState::MovingShield)
                {
                    let (sdx, sdy) = ft
                        .shield_destination
                        .or(ft.target_pos)
                        .map(|p| (p.x - elem.position_map().x, p.y - elem.position_map().y))
                        .unwrap_or((dx, dy));
                    let dist_sq = sdx * sdx + sdy * sdy;
                    speed_factors[idx] = if dist_sq < 25.0 {
                        1.0
                    } else if dist_sq < 100.0 {
                        1.5
                    } else {
                        2.0
                    };
                }
            }
            let speed_factor = speed_factors[idx];
            // Dispatch by order action: transition-animation orders
            // route to `MotionMethod::TillLastFrame`, while walking /
            // running orders route to `MotionMethod::Walk` (or
            // `MotionMethod::Fast` for RUNNING_WITH_SWORD).  The
            // TillLastFrame branch advances the order on animation
            // loop (`Terminated`) rather than on position arrival,
            // which is the right semantics for zero-distance pose
            // changes whose destination is already the actor's current
            // position.
            // Distance-producing movement animations use Walk/Fast.
            // Everything else (transitions, posture-changes, misc)
            // dispatched via tick_move maps to TillLastFrame.
            let is_movement_anim = order_uses_distance_motion(order_action);
            let is_transition_anim = !is_movement_anim;
            let fast_sword_motion = action_state == crate::element::ActionState::MovingFastSword
                || order_action == OrderType::RunningWithSword
                || door_pass_anim == Some(OrderType::RunningWithSword);
            let fast_climb_motion = matches!(
                anim,
                OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast
            );
            let motion_method = if is_transition_anim {
                MotionMethod::TillLastFrame
            } else if fast_sword_motion || fast_climb_motion {
                MotionMethod::Fast
            } else {
                MotionMethod::Walk
            };
            if let Some(LiftAnimContext::OnClimb {
                lift_type,
                lift_direction,
                ..
            }) = lift_translations[idx]
            {
                match (anim, lift_type) {
                    (
                        OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast,
                        crate::sector::LiftType::Wall,
                    )
                    | (
                        OrderType::ClimbingLadderUp
                        | OrderType::ClimbingLadderDown
                        | OrderType::ClimbingLadderUpFast
                        | OrderType::ClimbingLadderDownFast,
                        crate::sector::LiftType::Ladder,
                    ) => elem.set_direction_instantly(lift_direction),
                    _ => {}
                }
            }
            match (anim, door_pass_anim) {
                (
                    OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnWall;
                }
                (
                    OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::Flying;
                }
                (
                    OrderType::TransitionClimbingWallUpWaitingCrouched
                    | OrderType::TransitionClimbingWallDownWaitingUpright,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnWall;
                }
                (
                    OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast,
                    Some(_),
                ) => {
                    elem.posture = crate::element::Posture::OnLadder;
                }
                _ => {}
            }
            if let Some(wall_dir) = door_pass_wall_directions[idx] {
                let dir = if matches!(
                    (anim, elem.posture),
                    (
                        OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                        crate::element::Posture::Flying
                    )
                ) {
                    (wall_dir + 8) & 15
                } else {
                    wall_dir
                };
                elem.set_direction_instantly(dir);
            }

            // Run a one-step rotation of facing toward the goal
            // direction immediately before `perform_motion`.  If the
            // facing already matches the goal it is a no-op.
            let still_turning = elem.sprite.position_iface.turn();
            let direction = elem.direction() as u16;
            // The "already at destination" short-circuit needs the
            // predicate routed into `perform_motion`.  Keep a 0.01
            // epsilon to absorb any prior anti-collision jitter that
            // may have nudged the actor a tiny fraction off the
            // order's recorded destination.
            let dest_already_at_pos = motion_method != MotionMethod::TillLastFrame && dist <= 0.01;
            let motion_order = order_id.map(|order_id| MotionOrderContext {
                order_id,
                destination: goal,
                reverse: order_reverse,
                tolerance: order_tolerance,
                directional_tolerance: active_move_flags
                    .contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE),
                compute_direction: order_compute_direction,
                next_destination_same_action,
            });
            let sprite = &mut elem.sprite;
            let (motion_state, frame_dist_raw) = sprite.perform_motion(
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                direction,
                FrameProgression::Default,
                false,
                motion_method,
                dest_already_at_pos,
            );
            if let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, motion_state)
            {
                movement_state_effects.push((EntityId(idx as u32), posture, action_state));
            }
            if matches!(motion_state, MotionState::Start) && is_sword_motion {
                sword_movement_starts.push(EntityId(idx as u32));
            }
            if door_pass_anim.is_some()
                && matches!(motion_state, MotionState::Done)
                && matches!(
                    anim,
                    OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                )
            {
                door_pass_transition_done_effects.push(EntityId(idx as u32));
            }
            if matches!(motion_state, MotionState::Terminated) && is_sword_motion {
                sword_movement_terminations.push(EntityId(idx as u32));
            }
            if active_move_flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE)
                && anim == OrderType::RunningUpright
            {
                let frame_count = sprite.num_frames_for_anim(OrderType::RunningUpright);
                let cur = sprite.current_frame;
                if frame_count >= 2 && (cur == frame_count / 2 - 1 || cur == frame_count - 1) {
                    galopp_events.push(EntityId(idx as u32));
                }
            }
            // Turn-slowdown: when the sprite is still rotating toward
            // its goal direction, the per-tick walking distance is
            // multiplied by 0.6× (with a 0.7-unit floor to keep the
            // actor above the IsBlocked threshold).  The drunken-
            // soldier branch that *increases* the factor to 2.0× is
            // omitted here — this walking loop doesn't run
            // `MotionMethod::Drunken`.
            let mut frame_dist = frame_dist_raw;
            if still_turning && frame_dist > 0.0 {
                frame_dist *= 0.6;
                if frame_dist < 0.7 {
                    frame_dist = 0.7;
                }
            }
            let speed = frame_dist * speed_factor;

            if door_pass_anim.is_some()
                && matches!(
                    anim,
                    OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                        | OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                )
            {
                let goal_dir = elem.sprite.position_iface.get_direction_goal().as_u8();
                tracing::debug!(
                    entity = ?EntityId(idx as u32),
                    ?anim,
                    posture = ?elem.posture,
                    action_state = ?action_state,
                    dir = elem.direction(),
                    goal_dir,
                    row = elem.sprite.current_row,
                    frame = elem.sprite.current_frame,
                    pos_x = elem.position_map().x,
                    pos_y = elem.position_map().y,
                    pos3_y = elem.position().y,
                    pos3_z = elem.position().z,
                    speed,
                    dist,
                    "DoorPass movement state"
                );
            }

            // TillLastFrame branch: transition animations advance via
            // the animation-loop `Terminated` edge, not via position
            // arrival.  Still update position by the sprite's
            // per-frame distance along the vector toward the order's
            // target — end-of-run transitions carry ~26 units of
            // distance and must actually move the actor to reach the
            // goal (without this advance, soldiers stop at the
            // running-phase endpoint and never close the final ~26u
            // gap, leaving them outside sword_range forever and unable
            // to trigger begin_swordfight).  We skip the full walk
            // body below (anti-collision, arrival-pop, line-crossing)
            // since those apply to waypoint-driven walks.
            //
            // C++ seeds `PositionInterface` at the start of every new
            // sprite motion order and moves transition animations via
            // `UpdatePositionMap(fDistance)`, so this branch uses the
            // same precomputed map increment instead of a separate
            // dx/dy step.
            if is_transition_anim {
                let transition_has_map_target = goal.x != 0.0 || goal.y != 0.0;
                if !transition_has_map_target && !is_in_place_movement_transition(order_action) {
                    panic!(
                        "movement transition {:?} for entity {:?} has zero map target; refusing to treat (0,0) as an implicit destination",
                        order_action,
                        EntityId(idx as u32)
                    );
                }
                if transition_has_map_target && speed > 0.0 && dist > 0.01 {
                    let elem = entity.element_data_mut();
                    let pos = elem.position_map();
                    let pi = &mut elem.sprite.position_iface;
                    if !pi.is_increment_map_computed() {
                        tracing::warn!(
                            entity = ?EntityId(idx as u32),
                            order = ?order_action,
                            pos_x = pos.x,
                            pos_y = pos.y,
                            goal_x = goal.x,
                            goal_y = goal.y,
                            "movement transition lost cached map increment; recomputing from order target"
                        );
                        // TODO: identify which mid-order state mutation clears
                        // PositionInterface::computed_increment here. The order
                        // target is authoritative, so reseeding mirrors the
                        // new-order setup instead of advancing with stale data.
                        pi.set_map_goal(crate::coordinates::MapPoint {
                            x: goal.x,
                            y: goal.y,
                        });
                        pi.set_reversed_movement(order_reverse);
                        pi.set_tolerance(
                            order_tolerance,
                            active_move_flags
                                .contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE),
                        );
                        pi.set_goal_next_valid(false);
                        pi.compute_increment_all(order_compute_direction);
                    }
                    pi.update_position_map_scaled(speed);
                    elem.update_grid_cell();
                }
                if matches!(motion_state, MotionState::Terminated) {
                    let eid = EntityId(idx as u32);
                    if is_final_waypoint
                        && is_sword_motion
                        && let Some(human) = entity.human_data_mut()
                    {
                        human.last_motion_was_step_back_in_combat = active_move_flags
                            .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                    }
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    // Pop via the element we actually dispatched (`move_seq_id` /
                    // `move_elem_idx`), not `actor.active_movement.sequence_id`
                    // — the latter can be stale/None when the Move element was
                    // launched by the AI without setting active_movement
                    // (soldier chase paths).
                    order_pops.push((move_seq_id, move_elem_idx));
                    // Last order of the Move element just completed — flip
                    // back to Waiting and clear the active movement.
                    // Matches the `DoorPassAdvance::Done` arm below but for
                    // the transition-terminated path.
                    if is_final_waypoint {
                        let advance = if actor.active_door_pass.is_some() {
                            Self::advance_door_pass(
                                actor,
                                eid,
                                &mut door_triggers,
                                &mut select_triggers,
                                &mut self.next_order_id,
                            )
                        } else {
                            DoorPassAdvance::Done { completed: None }
                        };
                        match advance {
                            DoorPassAdvance::Continue {
                                destination,
                                action,
                                reverse,
                                compute_direction,
                                tolerance,
                            } => {
                                let order_id =
                                    crate::order::alloc_order_id(&mut self.next_order_id);
                                let mut order = crate::order::Order::new(
                                    action,
                                    destination.x,
                                    destination.y,
                                    order_id,
                                );
                                order.reverse = reverse;
                                order.compute_direction = compute_direction;
                                order.tolerance = tolerance;
                                transition_pushes.push((move_seq_id, move_elem_idx, order));
                            }
                            DoorPassAdvance::Paused { transition_order } => {
                                transition_pushes.push((
                                    move_seq_id,
                                    move_elem_idx,
                                    transition_order,
                                ));
                            }
                            DoorPassAdvance::Done { completed } => {
                                if let Some((door_index, direct)) = completed {
                                    completed_door_passes.push((eid, door_index, direct));
                                }
                                arrived.push((eid, actor.active_movement));
                                actor.clear_path();
                                actor.action_state =
                                    if is_swordfighting || actor.action_state.is_sword() {
                                        crate::element::ActionState::WaitingSword
                                    } else {
                                        crate::element::ActionState::Waiting
                                    };
                                actor.active_movement.clear();
                                actor.active_door_pass = None;
                            }
                            DoorPassAdvance::NoActive => {
                                tracing::warn!(
                                    entity = ?eid,
                                    "DoorPass: transition-terminated movement lost active pass"
                                );
                            }
                        }
                    }
                }
                continue;
            }

            if speed <= 0.0 {
                continue;
            }

            tracing::trace!(
                "tick_move: entity={:?} pos=({:.0},{:.0}) goal=({:.0},{:.0}) speed={speed:.1} action={:?} state={:?}",
                EntityId(idx as u32),
                elem.position_map().x,
                elem.position_map().y,
                goal.x,
                goal.y,
                anim,
                action_state,
            );

            // Snapshot the pre-move position + layer + posture so we
            // can run the elevation-line-crossing check after the
            // position is updated.  Pre-checks (must be moving on
            // map, not flying, not carried, inside grid) map to:
            // `dist > 0`, posture not Flying/Carried-ish, position
            // inside `fast_grid.level.map_bbox`.
            let old_pos_geo = crate::geo2d::pt(elem.position_map().x, elem.position_map().y);
            let entity_layer = elem.layer();
            let entity_posture = elem.posture;
            let eligible_for_crossing =
                !matches!(
                    entity_posture,
                    crate::element::Posture::Flying
                        | crate::element::Posture::OnWall
                        | crate::element::Posture::OnLadder
                        | crate::element::Posture::Carried
                        | crate::element::Posture::OnShoulders
                        | crate::element::Posture::CarryingCorpse
                        | crate::element::Posture::CarryingOnShoulders
                        | crate::element::Posture::HelpingToClimb
                ) && self.fast_grid.level.map_bbox.contains_point(old_pos_geo);

            // LINE-snap early arrival: when the active element
            // carries `MoveFlags::LINE` + `line_id` and the actor is
            // on the final waypoint, the arrival check uses distance
            // to the line (not the midpoint) against the element's
            // tolerance — snaps to the nearest line point instead of
            // the stored destination.
            let line_snap_arrival = if let Some((line_idx, tol)) = line_snaps[idx] {
                if !is_final_waypoint {
                    false
                } else {
                    let actor_pt = elem.position_map();
                    match self.fast_grid.level.jump_lines.get(usize::from(line_idx)) {
                        Some(jl) => {
                            let d = jl.compute_distance(actor_pt);
                            // Tolerance of 0 means "on the line" —
                            // accept a small epsilon so we don't
                            // stall when the actor approaches from a
                            // perpendicular angle.
                            let effective = if tol <= 0.0 { 1.0 } else { tol };
                            d <= effective
                        }
                        None => false,
                    }
                }
            } else {
                false
            };

            // Seek-arrival predicate:
            //
            //   - dist_sq = squared distance (target - pos), with Y
            //     stretched by the inverse aspect ratio (≈1.7434)
            //     when DIRECTIONAL_TOLERANCE is set (used for net
            //     pickup).
            //   - Arrive iff target.sector == self.sector AND
            //     dist_sq < tolerance² × 1.1025 (the "5% tolerance"
            //     margin baked into the squared comparison).
            //
            // The check runs every tick (not just the last waypoint),
            // so a moving target that wanders into range mid-route
            // ends the seek immediately and the post-seek sequence
            // fires.  The pre-pass only populates `FinalTol` for
            // SEEK-flagged movements with a resolvable target (entity
            // or shield destination), so `ft.tol > 0` is the
            // live-seek gate; non-seek elements skip this branch
            // entirely and fall through to the standard `dist <=
            // speed` arrival.  USE_POINT samples the target's current
            // hotspot; SEEK_SHIELD uses the movement element
            // destination.
            let ft = final_tolerances[idx];
            let point_seek_post_arrival = is_final_waypoint
                && dist <= speed
                && point_seek_post_sectors[idx]
                    .map(|seek_sector| elem.sector() == Some(seek_sector))
                    .unwrap_or(false);
            let tolerance_arrival = ft.tol > 0.0 && {
                let self_sector = elem.sector();
                // Require same sector.
                if ft.target_sector.is_some() && self_sector != ft.target_sector {
                    false
                } else {
                    // Check the live seek target here, not the
                    // movement order's static waypoint.  That matters
                    // for sword-strike seeks: the target can shift
                    // during the approach, and the PC must stop at
                    // the seek-distance from the target, not from the
                    // stale path endpoint.  Either `shield_destination`
                    // or `target_pos` is guaranteed to be `Some` by
                    // the FinalTol pre-pass; the `expect` documents
                    // that invariant rather than papering over a
                    // missing target with the order's goal vector
                    // (which would wrongly arrive at intermediate
                    // waypoints).
                    let target = ft
                        .shield_destination
                        .or(ft.target_pos)
                        .expect("SEEK FinalTol must have shield_destination or target_pos");
                    let (target_dx, target_dy) = (
                        target.x - elem.position_map().x,
                        target.y - elem.position_map().y,
                    );
                    let (dx_use, dy_use) = if let Some(off) = ft.use_point_offset {
                        (target_dx + off.x, target_dy + off.y)
                    } else {
                        (target_dx, target_dy)
                    };
                    let dy_effective = if ft.directional {
                        const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                        dy_use * INVERSE_ASPECT_RATIO
                    } else {
                        dy_use
                    };
                    let dist_sq = dx_use * dx_use + dy_effective * dy_effective;
                    // 5% tolerance: 1.05² = 1.1025.  Squared-norm
                    // comparison avoids the sqrt that `dist` computed
                    // above.
                    dist_sq < ft.tol * ft.tol * 1.1025
                }
            };

            // FROZEN stand-still wait.  When the seek arrival
            // predicate fires at an intermediate waypoint and there
            // is no `post_seek_sequence` to consume the arrival, the
            // actor freezes its sprite frame in place near the target
            // until either the target moves out of tolerance
            // (next-tick `tick_refresh_seeks` detects drift and
            // rebuilds the path) or a post-seek is later attached.
            // We honour this by simply skipping the per-tick movement
            // step (no order pop, no position update, no sprite
            // advance) so the actor's position + orders persist for
            // the next tick to re-evaluate.
            //
            // This branch only fires for entity-target seeks without
            // a queued post-seek interaction (e.g. AI follow seeks
            // built outside `apply_interaction_with_seek`).  The
            // common PC interaction path always carries a post-seek
            // and routes through the `start_post_seek` branch below
            // instead.
            let frozen_seek_wait = tolerance_arrival && !is_final_waypoint && !ft.has_post_seek;
            if frozen_seek_wait {
                tracing::trace!(
                    entity = ?EntityId(idx as u32),
                    "tick_move: FROZEN seek wait (target in range, no post-seek, mid-path)",
                );
                continue;
            }

            let order_tolerance_arrival = order_tolerance > 0.0 && dist > speed && {
                let nx = dx / dist;
                let ny = dy / dist;
                let remaining_x = dx - nx * speed;
                let remaining_y = dy - ny * speed;
                let remaining_along_heading = if active_move_flags
                    .contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE)
                {
                    const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                    nx * remaining_x + ny * remaining_y * INVERSE_ASPECT_RATIO
                } else {
                    nx * remaining_x + ny * remaining_y
                };
                remaining_along_heading <= order_tolerance
            };

            if dist <= speed || line_snap_arrival || tolerance_arrival || order_tolerance_arrival {
                // Reached waypoint — snap to it and advance
                if order_tolerance_arrival {
                    let nx = dx / dist;
                    let ny = dy / dist;
                    let mut pm = elem.position_map();
                    pm.x += nx * speed;
                    pm.y += ny * speed;
                    elem.set_position_map(pm);
                } else if !line_snap_arrival && !tolerance_arrival {
                    elem.set_position_map(crate::coordinates::MapPoint {
                        x: goal.x,
                        y: goal.y,
                    });
                }
                // On line-snap arrival leave `position_map` where it
                // is; the actor is already "on the line" within
                // tolerance and should not teleport to the
                // midpoint.

                let eid = EntityId(idx as u32);

                // Transition-animation refresh.  When this arrival
                // pop will leave a transition-to-waiting order as the
                // next (final) order in the queue AND the seek target
                // has drifted since the last refresh AND the new
                // distance is greater than `(transition_distance +
                // seek_distance) * 1.05`, abort the queued transition
                // and call RefreshSeek immediately so the actor
                // doesn't play a transition that lands too far from
                // the (moved) target.  Computed here using the
                // still-live `elem` borrow (sprite + position) before
                // the actor mut borrow takes over — NLL would
                // otherwise reject the second borrow.
                let transition_refresh_target: Option<(
                    crate::sequence::SequenceId,
                    usize,
                    OrderType,
                    MapPoint,
                    f32,
                )> = if !is_final_waypoint
                    && !tolerance_arrival
                    && ft.tol > 0.0
                    && let Some(target_now) = ft.target_pos
                {
                    let last = ft.last_seek_target_position;
                    let drifted = (target_now.x - last.x).abs() > 0.01
                        || (target_now.y - last.y).abs() > 0.01;
                    if drifted {
                        let seq_id = move_seq_id;
                        let elem_idx = move_elem_idx;
                        let next_anim = self
                            .sequence_manager
                            .get_element(seq_id, elem_idx)
                            .and_then(|e| e.orders.get(1).map(|o| o.order_type));
                        if matches!(
                            next_anim,
                            Some(OrderType::TransitionRunningUprightWaitingUpright)
                                | Some(OrderType::TransitionWalkingUprightWaitingUpright)
                                | Some(OrderType::TransitionWalkingCrouchedWaitingCrouched)
                        ) {
                            let next_anim = next_anim.unwrap();
                            let pos = elem.position_map();
                            let dx = target_now.x - pos.x;
                            let dy = target_now.y - pos.y;
                            let dy_eff = if ft.directional {
                                const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                                dy * INVERSE_ASPECT_RATIO
                            } else {
                                dy
                            };
                            let sq = dx * dx + dy_eff * dy_eff;
                            let trans_dist = if elem.sprite.has_animation(next_anim) {
                                elem.sprite.distance_for_animation(next_anim) as f32
                            } else {
                                0.0
                            };
                            let raw = trans_dist + ft.tol;
                            let threshold = raw * 1.05;
                            if sq > threshold * threshold {
                                Some((seq_id, elem_idx, next_anim, target_now, sq.sqrt()))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((seq_id, elem_idx, _next_anim, _target, _dist)) =
                    transition_refresh_target
                {
                    transition_seek_refreshes.push((eid, seq_id, elem_idx));
                    tracing::trace!(
                        ?eid,
                        ?_next_anim,
                        target_x = _target.x,
                        target_y = _target.y,
                        new_dist = _dist,
                        "tick_move: transition-animation refresh fired (target drifted beyond transition+seek_dist)",
                    );
                    continue;
                }

                let actor = entity.actor_data_mut().unwrap();
                // The post-seek sequence fires whenever the seek
                // arrival predicate is true and a post-seek sequence
                // is attached — no final-waypoint gate.  The
                // `tolerance_arrival` guard above already enforces the
                // post-seek requirement for intermediate waypoints, so
                // reaching this point with both flags set is the
                // "terminate the seek and launch the post-seek" path.
                let start_post_seek = (tolerance_arrival || point_seek_post_arrival)
                    && actor.post_seek_sequence.is_some();
                let start_post_seek = if start_post_seek && actor.active_door_pass.is_some() {
                    tracing::warn!(
                        entity = ?eid,
                        "DoorPass: suppressing post-seek teardown during active pass"
                    );
                    false
                } else {
                    start_post_seek
                };

                // Waypoint reached — queue a `do_next_order` pop on
                // the actor's Move element.
                if start_post_seek {
                    post_seek_arrivals.push((eid, move_seq_id, move_elem_idx));
                } else {
                    order_pops.push((move_seq_id, move_elem_idx));
                }

                if start_post_seek {
                    actor.clear_path();
                    actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                        crate::element::ActionState::WaitingSword
                    } else {
                        crate::element::ActionState::Waiting
                    };
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    if is_sword_motion && let Some(human) = entity.human_data_mut() {
                        human.last_motion_was_step_back_in_combat = active_move_flags
                            .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                    }
                    continue;
                }

                if is_final_waypoint {
                    // All waypoints for current walk step consumed.
                    // Check if we have more door-pass steps.
                    let advance = if actor.active_door_pass.is_some() {
                        Self::advance_door_pass(
                            actor,
                            eid,
                            &mut door_triggers,
                            &mut select_triggers,
                            &mut self.next_order_id,
                        )
                    } else {
                        DoorPassAdvance::Done { completed: None }
                    };

                    match advance {
                        DoorPassAdvance::Continue {
                            destination,
                            action,
                            reverse,
                            compute_direction,
                            tolerance,
                        } => {
                            // Push a walking order for the new Walk
                            // step onto the actor's current sequence
                            // element, to be installed after the
                            // entity loop closes (same deferred
                            // mechanism as Transition steps).
                            let order_id = crate::order::alloc_order_id(&mut self.next_order_id);
                            let mut order = crate::order::Order::new(
                                action,
                                destination.x,
                                destination.y,
                                order_id,
                            );
                            order.reverse = reverse;
                            order.compute_direction = compute_direction;
                            order.tolerance = tolerance;
                            transition_pushes.push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Paused { transition_order } => {
                            // Transition animation queued — push the
                            // order onto the actor's current sequence
                            // element after the loop closes.
                            transition_pushes.push((move_seq_id, move_elem_idx, transition_order));
                        }
                        DoorPassAdvance::Done { completed } => {
                            if let Some((door_index, direct)) = completed {
                                completed_door_passes.push((eid, door_index, direct));
                            }
                            // Final waypoint's do_next_order pop was
                            // already collected above when
                            // `path_waypoint_index` advanced past the
                            // end of the list; that pop will either
                            // drain the Move element entirely
                            // (triggering `element_terminated` +
                            // `ensure_wait_element` internally) or
                            // leave an end-transition order as the
                            // new current, which the animation driver
                            // will play next tick.
                            arrived.push((eid, actor.active_movement));
                            actor.clear_path();
                            // Flip action_state to Waiting so any
                            // pending end-transition order on the
                            // Move element gets picked up by the
                            // animation driver (gated on
                            // `!is_moving()`).  Preserves sword state
                            // for combat exits.
                            actor.action_state =
                                if is_swordfighting || actor.action_state.is_sword() {
                                    crate::element::ActionState::WaitingSword
                                } else {
                                    crate::element::ActionState::Waiting
                                };
                            actor.active_movement.clear();
                            actor.active_door_pass = None;
                            if is_sword_motion && let Some(human) = entity.human_data_mut() {
                                human.last_motion_was_step_back_in_combat = active_move_flags
                                    .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                            }
                        }
                        DoorPassAdvance::NoActive => {
                            tracing::warn!(
                                entity = ?eid,
                                "DoorPass: final waypoint reached but active pass was already gone"
                            );
                        }
                    }
                }
            } else {
                // Move toward waypoint.
                //
                // Actor-vs-actor anti-collision: deviate around other
                // actors' repulsive zones before committing the step.
                // Runs between the motion advance and the position
                // commit, gated on the mover's `anti_collision_on`
                // flag — the flag stays `true` by default so this is
                // active for every normal walk.
                let nx = dx / dist;
                let ny = dy / dist;
                let anti_on = entity.position_iface().is_anti_collision_on();
                // Pull transient anti-collision context from position_iface
                // (move box, half-diagonal) + the current path goal.  The
                // persistent state (deviated / blocked_count / box_blocked /
                // radius) lives on the actor's PI directly now.
                let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                let (move_box, half_diagonal) = {
                    let pi = entity.position_iface();
                    (*pi.get_move_box(), pi.get_half_diagonal())
                };

                let (dx_step, dy_step) = if let Some(Some(mover_snap)) = anti_snapshots.get(idx) {
                    let pi = entity.position_iface_mut();
                    let mut state = super::anti_collision::AntiCollisionState {
                        pi,
                        move_box,
                        half_diagonal,
                        goal_map,
                    };
                    super::anti_collision::apply_anti_collision_step(
                        mover_snap,
                        &anti_snapshots,
                        &self.ai_global.repulsive_points,
                        Some(&self.fast_grid),
                        Some(&mut state),
                        nx,
                        ny,
                        speed,
                        anti_on,
                    )
                } else {
                    (nx * speed, ny * speed)
                };
                let new_pos_x;
                let new_pos_y;
                {
                    let elem = entity.element_data_mut();
                    let mut pm = elem.position_map();
                    pm.x += dx_step;
                    pm.y += dy_step;
                    elem.set_position_map(pm);
                    new_pos_x = pm.x;
                    new_pos_y = pm.y;
                }

                // Water splash titbit emission.  Every walk tick
                // where `speed > 2` and the actor's cached material
                // is water, the sprite's splatter counter ticks up;
                // on `>= 2` a water particle is added at the actor's
                // 3D position and the counter resets.  Cosmetic but
                // observable — actors crossing a stream kick up
                // splash titbits.
                {
                    let elem = entity.element_data_mut();
                    if speed > 2.0 && elem.material() == crate::element::GameMaterial::Water {
                        if elem.sprite.splitch_count >= 2 {
                            elem.sprite.splitch_count = 0;
                            let pos = elem.position();
                            let layer = elem.layer();
                            water_splash_emits.push((
                                EntityId(idx as u32),
                                crate::coordinates::WorldPoint3D {
                                    x: pos.x,
                                    y: pos.y,
                                    z: pos.z,
                                },
                                layer,
                            ));
                        } else {
                            elem.sprite.splitch_count = elem.sprite.splitch_count.saturating_add(1);
                        }
                    }
                }

                // When the blocked counter trips, the motion aborts
                // and the backing sequence element is marked
                // Impossible.
                if entity.position_iface().is_blocked() {
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    if let Some(seq_id) = actor.active_movement.sequence_id {
                        blocked_impossible.push((seq_id, actor.active_movement.element_index));
                    }
                    let restore_anti_collision = {
                        let restore_anti_collision = actor.active_door_pass.is_some();
                        if restore_anti_collision {
                            tracing::warn!(
                                entity = ?EntityId(idx as u32),
                                "DoorPass: movement blocked; clearing active pass with aborted movement"
                            );
                            actor.active_door_pass = None;
                        }
                        actor.clear_path();
                        actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                            crate::element::ActionState::WaitingSword
                        } else {
                            crate::element::ActionState::Waiting
                        };
                        actor.active_movement.clear();
                        restore_anti_collision
                    };
                    if restore_anti_collision {
                        entity.position_iface_mut().set_anti_collision_on(true);
                    }
                    entity.position_iface_mut().reset_box_blocked();
                }

                // Sync the just-moved position back into the snapshot
                // so later actors in this tick see the serial
                // "already-moved" position of this one.  Without this
                // two actors heading for the same cell both see each
                // other at the *old* position and can still overlap.
                if let Some(Some(snap)) = anti_snapshots.get_mut(idx) {
                    let new_pos = crate::geo2d::pt(new_pos_x, new_pos_y);
                    snap.position_map = new_pos.into();
                    if let Some(rp) = snap.repulsive_point.as_mut() {
                        rp.position = new_pos.into();
                    }
                    for rp in snap.extra_repulsive_points.iter_mut() {
                        // Animal front/back points are offsets from
                        // the torso along facing direction — rebuild
                        // them from the post-move torso.  For humans
                        // this loop is empty so the branch is cheap.
                        rp.position.x += dx_step;
                        rp.position.y += dy_step;
                    }
                    for rl in snap.repulsive_lines.iter_mut() {
                        rl.a.x += dx_step;
                        rl.a.y += dy_step;
                        rl.b.x += dx_step;
                        rl.b.y += dy_step;
                    }
                }
            }

            // Queue an elevation-line-cross check for this tick. The
            // actual fast-grid query + obstacle swap runs after the
            // loop, since `check_for_line_crossing` needs `&mut self`.
            //
            // Also queue a patch-line-cross check for PC actors —
            // LINE_PATCH handling is gated to PCs only.
            if eligible_for_crossing {
                let new_pos = entity.element_data().position_map();
                let new_pos_geo = crate::geo2d::pt(new_pos.x, new_pos.y);
                if self.fast_grid.level.map_bbox.contains_point(new_pos_geo) {
                    let old_pos = MapPoint::from_geo(old_pos_geo);
                    line_cross_checks.push((EntityId(idx as u32), old_pos, new_pos, entity_layer));
                    if entity.is_pc() {
                        patch_cross_checks.push((
                            EntityId(idx as u32),
                            old_pos,
                            new_pos,
                            entity_layer,
                        ));
                    }
                    // LINE_SOUND crossing is not gated on PC — every
                    // moving actor refreshes its `material` on
                    // crossing a sound-material boundary so footstep
                    // sound playback picks the right per-frame
                    // material.
                    sound_cross_checks.push((EntityId(idx as u32), old_pos, new_pos, entity_layer));
                }
            }
        }

        for entity_id in sword_movement_starts {
            self.apply_sword_movement_start_initiative_transfer(entity_id);
        }
        for (entity_id, posture, action_state) in movement_state_effects {
            if let Some(entity) = self.get_entity_mut(entity_id) {
                entity.set_posture(posture);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = action_state;
                }
            }
        }
        for entity_id in door_pass_transition_done_effects {
            self.apply_door_pass_transition_done_side_effects(assets, entity_id);
        }
        for entity_id in sword_movement_terminations {
            self.maybe_provoke_after_sword_movement_terminated(assets, entity_id);
        }

        // Dispatch elevation-line crossings detected during the loop.
        // Runs as a post-pass after the per-actor movement update.
        // When a human actor crosses an elevation line, we also fire
        // `UpdateRoll` so any in-progress Rolling combat_anim can
        // re-aim its flight at the new obstacle's slope.
        for (entity_id, old_pos, new_pos, layer) in line_cross_checks {
            let crossed = self.check_for_line_crossing(assets, entity_id, old_pos, new_pos, layer);
            if crossed {
                let is_human = self
                    .get_entity(entity_id)
                    .map(|e| e.is_human())
                    .unwrap_or(false);
                if is_human {
                    self.update_roll_after_crossing(assets, entity_id);
                }
                let compute_direction = self
                    .sequence_manager
                    .in_progress_element_for_actor_matching(entity_id, |e| e.data.is_movement())
                    .and_then(|(seq_id, elem_idx)| {
                        self.sequence_manager
                            .get_element(seq_id, elem_idx)
                            .and_then(|e| e.current_order())
                    })
                    .map(|order| order.compute_direction);
                if let Some(compute_direction) = compute_direction
                    && let Some(entity) = self.get_entity_mut(entity_id)
                {
                    entity
                        .position_iface_mut()
                        .compute_increment_all(compute_direction);
                }
            }
        }

        // Dispatch patch-line (LINE_PATCH) crossings for PCs.  On
        // crossing a LINE_PATCH line, route the PC's new position
        // through the patch's Enter/Leave + auto-Apply flow.
        for (entity_id, old_pos, new_pos, layer) in patch_cross_checks {
            self.check_for_patch_line_crossing(assets, entity_id, old_pos, new_pos, layer);
        }

        // Dispatch transition-animation seek refreshes detected
        // during the per-tick movement loop.  Fires `RefreshSeek`
        // immediately when a queued transition-to-waiting order would
        // land too far from a moved target.  Same machinery as
        // `tick_refresh_seeks` — re-resolve the seek destination,
        // build a fresh single-element seek sequence, and re-launch
        // via `relaunch_seek_replacement`.  Runs before the LINE_SOUND
        // dispatch so the relaunched seek's first dispatch tick
        // already sees the freshly-built grid line state.
        for (owner, seq_id, elem_idx) in transition_seek_refreshes {
            // Re-read the seek element's flags / target / tolerance /
            // action because the dispatch above might have mutated
            // state on adjacent elements.  When the element no longer
            // looks like an entity-target seek, skip silently.
            let snapshot = self
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| match &e.data {
                    crate::sequence::SequenceElementData::Movement {
                        flags,
                        element,
                        tolerance,
                        action,
                        ..
                    } => element.map(|t| (*flags, t, *tolerance, *action)),
                    _ => None,
                });
            if let Some((flags, target, tolerance, action)) = snapshot {
                let new_target_pos = self
                    .get_entity(target)
                    .map(|e| e.element_data().position_map())
                    .unwrap_or_default();
                self.apply_seek_refresh(
                    assets,
                    owner,
                    seq_id,
                    elem_idx,
                    target,
                    action,
                    flags,
                    tolerance,
                    new_target_pos,
                );
            }
        }

        // Dispatch sound-line (LINE_SOUND) crossings for every actor.
        // On crossing a LINE_SOUND line, refresh the actor's
        // `material` from the new SECTOR_SOUND polygon containment
        // (or fall back to the obstacle / default material).  Drives
        // footstep sound playback parity.
        for (entity_id, old_pos, new_pos, layer) in sound_cross_checks {
            self.check_for_sound_line_crossing(assets, entity_id, old_pos, new_pos, layer);
        }

        // Execute pending door-pass triggers (PassingDoor steps).
        // These need &mut self for layer/sector changes and building callbacks.
        for (entity_id, door_index, direct, trigger_num) in door_triggers {
            self.execute_pass_door(assets, entity_id, door_index, direct, trigger_num);
        }
        for (entity_id, door_index, direct) in completed_door_passes {
            tracing::debug!(
                entity = ?entity_id,
                door = %door_index,
                direct,
                "DoorPass: completed"
            );
            self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
        }

        // Push queued door-pass Transition orders onto each actor's
        // current sequence element.  The current order list — the
        // transition order blocks subsequent orders until its sprite
        // animation completes.
        for (seq_id, elem_idx, order) in transition_pushes {
            if self
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|elem| elem.command == crate::element::Command::PassDoor)
                && let Some(owner) = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|elem| elem.owner)
            {
                self.apply_door_pass_continue_state(owner, order.order_type);
            }
            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        }

        // Fire pending Select hulk flashes.
        for (entity_id, speed) in select_triggers {
            self.apply_select_hulk(entity_id, speed);
        }

        for (entity_id, seq_id, elem_idx) in post_seek_arrivals {
            self.start_post_seek_sequence(entity_id, Some((seq_id, elem_idx)));
        }

        // Drain collected waypoint pops against each actor's Move
        // element.  One pop per waypoint-arrival (both intermediate
        // and final).  When the final pop empties the order queue,
        // `do_next_order` internally calls `element_terminated` +
        // `ensure_wait_element` to transition the sequence element to
        // Terminated on queue exhaustion.  When an end-transition
        // order was spliced in by `post_process_path`, the final
        // walking pop leaves the end-transition as the new current
        // and the animation driver plays it; its own `do_next_order`
        // on completion then terminates the element.
        for (seq_id, elem_idx) in order_pops {
            self.do_next_order(seq_id, elem_idx);
        }

        // Drain water-splash titbit emissions queued from the walk
        // branch.  Emits a water particle at the actor's 3D position
        // with no element supplier.
        for (_eid, position, layer) in water_splash_emits {
            self.titbit_manager.add_titbit(
                position,
                layer,
                crate::titbit::TitbitKind::Water,
                crate::titbit::ElementHandle::INVALID,
                0,
                crate::titbit::ElementHandle::INVALID,
                false,
                crate::titbit::INVALID_ID,
                true, // display_titbits_enabled — config plumbing not threaded through this site yet
                None,
                None,
            );
        }
        for (seq_id, elem_idx) in blocked_impossible {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
        }

        // Collect entity IDs for EventReachPoint dispatch.  Two paths
        // fire the same event: the condolation drain (triggered by
        // `element_terminated` above) and
        // `dispatch_reach_point_events` called from the caller after
        // this function returns.  Keep the explicit return so callers
        // that don't yet rely on the condolation-side dispatch (and
        // for now to preserve existing event-timing) still receive
        // the arrival list.
        let mut reach_point_entities: Vec<EntityId> = Vec::new();
        for (entity_id, _am) in &arrived {
            reach_point_entities.push(*entity_id);
        }
        (reach_point_entities, galopp_events)
    }

    /// Advance through door-pass steps after a walk step completes.
    ///
    /// Processes `PassingDoor` triggers immediately, pauses the actor
    /// for `Transition` steps (animation plays in place via
    /// `active_ai_anim` with [`AiAnimCompletion::ResumeDoorPass`]), and
    /// starts the next `Walk` step if one exists.
    ///
    /// See [`DoorPassAdvance`] for return semantics.
    pub(super) fn advance_door_pass(
        actor: &mut crate::element::ActorData,
        entity_id: EntityId,
        door_triggers: &mut Vec<(EntityId, crate::gate::DoorIndex, bool, u8)>,
        select_triggers: &mut Vec<(EntityId, f32)>,
        next_order_id: &mut u32,
    ) -> DoorPassAdvance {
        // Process non-Walk steps until we hit a Walk, a Transition
        // (pause), or run out.
        loop {
            let dp = match actor.active_door_pass.as_mut() {
                Some(dp) => dp,
                None => {
                    tracing::warn!(
                        entity = ?entity_id,
                        "DoorPass: advance requested without active pass"
                    );
                    return DoorPassAdvance::NoActive;
                }
            };
            let step = match dp.steps.pop_front() {
                Some(s) => s,
                None => {
                    // All steps consumed — door pass complete.
                    let completed = Some((dp.door_index, dp.direct));
                    actor.active_door_pass = None;
                    return DoorPassAdvance::Done { completed };
                }
            };

            match step {
                crate::element::DoorPassStep::PassingDoor => {
                    let dp = actor.active_door_pass.as_mut().unwrap();
                    let trigger_num = dp.triggers_fired;
                    dp.triggers_fired += 1;
                    door_triggers.push((entity_id, dp.door_index, dp.direct, trigger_num));
                    // Continue processing next step.
                }
                crate::element::DoorPassStep::Select { speed } => {
                    // Select handler: `StartHulk(OCN_DEFAULT, 2, true,
                    // tolerance)` on self and, if carrying, on the
                    // carried element.  The actual start_hulk call
                    // needs access to the carrier's HumanData and the
                    // carried entity, so we record the request here
                    // and let the caller apply it outside the borrow
                    // on `actor`.
                    select_triggers.push((entity_id, speed));
                    // Continue to the next step immediately — Select
                    // returns Terminated.
                }
                crate::element::DoorPassStep::Transition { action, reverse } => {
                    // The transition order sits at the front of the
                    // order queue and blocks subsequent orders until
                    // its sprite animation completes.  We build the
                    // transition order here and hand it back to the
                    // caller, who pushes it onto the actor's current
                    // sequence element.  `ResumeDoorPass` completion
                    // re-enters this function when the animation
                    // finishes.
                    //
                    // Save the walking action_state and flip to Waiting
                    // so `tick_entity_movement` stops advancing and the
                    // animation tick drives the transition sprite.
                    let saved = actor.action_state;
                    actor.action_state = crate::element::ActionState::Waiting;
                    actor.clear_path();
                    if let Some(dp) = actor.active_door_pass.as_mut() {
                        dp.saved_action_state = Some(saved);
                        dp.current_action = action;
                        dp.current_reverse = reverse;
                    }
                    let order_id = crate::order::alloc_order_id(next_order_id);
                    let mut order = crate::order::Order::new(action, 0.0, 0.0, order_id);
                    order.reverse = reverse;
                    order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                    tracing::debug!(
                        entity = ?entity_id,
                        ?action,
                        reverse,
                        "DoorPass: pausing for Transition animation"
                    );
                    return DoorPassAdvance::Paused {
                        transition_order: order,
                    };
                }
                crate::element::DoorPassStep::Walk {
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                } => {
                    // Restore the pre-transition action_state if we were
                    // paused; the walk animation itself comes from
                    // `current_action` (read by tick_entity_movement via
                    // `door_pass_anim`) so the actor resumes moving with
                    // the correct sprite.
                    if let Some(dp) = actor.active_door_pass.as_mut() {
                        dp.current_action = action;
                        dp.current_reverse = reverse;
                        if let Some(saved) = dp.saved_action_state.take() {
                            actor.action_state = saved;
                        }
                    }
                    // Hand the Walk destination back to the caller —
                    // advance_door_pass doesn't have sequence_manager
                    // access, so it can't push the walking order
                    // directly onto the PassDoor element.  The caller
                    // (tick_entity_movement's post-loop door-pass
                    // dispatch) does the order push.
                    return DoorPassAdvance::Continue {
                        destination: destination.into(),
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                    };
                }
            }
        }
    }

    /// Runtime detector for Shape 1 contract violations — logs a warning
    /// (and fires a `debug_assert!`) when a movement intent is drained
    /// while the actor is still in a "waiting" substate that relies on
    /// an exit event the halt-teardown will suppress.
    ///
    /// The Shape 1 wrappers (`EnemyAi::go_to` et al.) force callers to
    /// commit a new substate before queuing a movement — if the current
    /// substate is still in the wedge-prone set at drain time, either a
    /// new caller bypassed the wrapper via `ai.base.go_to(...)` or an
    /// external code path queued the intent on the actor's behalf
    /// without a corresponding `set_state`.  In either case the halt
    /// below will swallow the exit event and leave the AI stranded.
    fn check_shape1_contract(&self, entity_id: EntityId) {
        let Some(Some(entity)) = self.entities.get(entity_id.0 as usize) else {
            return;
        };
        let Some(ai) = entity.ai_controller() else {
            return;
        };
        use crate::ai::Substate;
        let wedge_prone = matches!(
            ai.current_substate,
            Substate::AttackingSwordfightParade
                | Substate::AttackingReactiontime
                | Substate::AttackingReactiontimeTurning
                | Substate::AttackingReactiontimeBending
        );
        if wedge_prone {
            tracing::warn!(
                entity = entity_id.0,
                substate = ?ai.current_substate,
                "Shape 1 violation: movement intent drained while actor is in a \
                 wedge-prone substate — halt-teardown will swallow the exit event. \
                 Likely cause: a caller bypassed EnemyAi::go_to / ai.base.go_to, or \
                 queued a movement intent without calling set_state first."
            );
            debug_assert!(
                !wedge_prone,
                "Shape 1 violation at entity {} in substate {:?}",
                entity_id.0, ai.current_substate
            );
        }
    }

    /// Each tick, AI controllers may produce movement/action orders.
    /// This method drains them and submits corresponding path requests.
    ///
    /// `AiController::pending_halt` (set by `stop_all` / `FaceTo`) is
    /// drained inside [`Self::launch_pending_orders_for_npc`] so the
    /// halt happens on the same call stack as the new element launch,
    /// `StopAll` / `FaceTo` halt the actor inline.
    pub(super) fn process_pending_ai_orders(&mut self) {
        let npc_ids = self.npc_ids.clone();
        for npc_id in npc_ids {
            self.launch_pending_orders_for_npc(npc_id);
        }
    }

    /// Per-NPC half of [`Self::process_pending_ai_orders`] — drains one
    /// NPC's `pending_orders` queue and launches the corresponding
    /// movement / turn / generic sequences.  Called both from the
    /// top-of-tick global pass and from the per-NPC synchronous drain
    /// in [`EngineInner::dispatch_think_with_drain`] so `Face` / `GoTo`
    /// etc. take effect inside the same call stack as the handler that
    /// issued them — `Face` / `GoTo` launch the sequence inline.
    pub(super) fn launch_pending_orders_for_npc(&mut self, entity_id: EntityId) {
        // `StopAll` halts the actor inline before subsequent work,
        // and `FaceTo` / `GoTo` do the same on their own.  The Halt
        // is deferred to this drain (via `pending_halt`) so it runs
        // on the same tick as the pending-order launch.  Honor the
        // flag here — before launching new orders — so the
        // `Stop(Preference)` cascade interrupts any in-progress
        // sequence element (e.g. a yellow-? Turn mid-`bored-exit`)
        // and the new element launched below starts from a clean
        // slate.  `halt_actor` brackets the stop with
        // `inside_halt_method=true` so condolations queued by the
        // interrupt are tagged `from_halt` and don't fire
        // `Think(EventDone)`.
        let (has_pending_orders, take_halt) = {
            let Some(entity) = self
                .entities
                .get_mut(entity_id.0 as usize)
                .and_then(|e| e.as_mut())
            else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let halt = ai.pending_halt;
            ai.pending_halt = false;
            (ai.has_pending_orders(), halt)
        };
        if take_halt {
            self.halt_actor(entity_id);
        }
        if !has_pending_orders {
            return;
        }
        let intents: Vec<crate::order::AiOrderIntent> = {
            let Some(entity) = self
                .entities
                .get_mut(entity_id.0 as usize)
                .and_then(|e| e.as_mut())
            else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.take_pending_orders()
        };

        for intent in intents {
            match intent.order_type {
                OrderType::WalkingUpright
                | OrderType::RunningUpright
                | OrderType::WalkingCrouched
                | OrderType::WalkingAlerted
                | OrderType::RiderCharging => {
                    // `find_accessible` / `ask_obstacle` pre-flight
                    // gates.  Run them before the halt so a failure
                    // leaves the outgoing sequence in place rather
                    // than tearing it down only to abandon the new
                    // move.
                    let mut intent = intent;
                    if !self.preflight_ai_goto(entity_id, &mut intent) {
                        continue;
                    }
                    // AI `GoTo` calls `Halt()` before issuing the new
                    // movement unless `no_halt` was set.  This tears
                    // down the previous sequence at the Preference
                    // priority, with `inside_halt_method=true` to
                    // suppress the `Think(EventDone)` /
                    // `Think(EventImpossible)` /
                    // `Think(EventCouldntReachpoint)` that the
                    // interrupted element would otherwise fire.
                    if !intent.no_halt {
                        self.check_shape1_contract(entity_id);
                        self.halt_actor(entity_id);
                    }
                    self.launch_ai_move(entity_id, &intent);
                }
                OrderType::Turning => {
                    if !intent.no_halt {
                        self.halt_actor(entity_id);
                    }
                    // Face toward a position — launch a Turn sequence
                    // through the normal `Instruct → GenerateTransition`
                    // pipeline.  The Turn command's exit flags
                    // prepend a
                    // `TransitionWaitingUprightBoredWaitingUpright` at
                    // `EventSeesShadow` (yellow `?`) time so the
                    // guard is already in Waiting by EventView.
                    let order = intent.stamp(self.alloc_order_id());
                    self.launch_single_order_sequence_stamped_ex(
                        entity_id,
                        crate::element::Command::Turn,
                        order,
                        true,
                    );
                }
                _ => {
                    // Other order types go on their own single-order
                    // sequence for the animation driver to pick up.
                    let order = intent.stamp(self.alloc_order_id());
                    self.launch_single_order_sequence_stamped(
                        entity_id,
                        crate::element::Command::Generic,
                        order,
                    );
                }
            }
        }
    }
    // ─── Elevation-line crossing ──────────────────────────────────

    /// Find a projection-area sight obstacle on `layer` whose
    /// screen-space plane contains `pos`.
    ///
    /// Used by the elevation-line emergency fallbacks: iterate plane
    /// sectors in the spatial bucket at `(pos, layer 0)`, then keep
    /// the one whose attached sight obstacle's layer matches and whose
    /// screen-space sector plane contains the position.  We don't carry
    /// a plane-sector registry yet — but every plane sector wraps a
    /// single projection-area obstacle, so iterating projection-area
    /// obstacles directly gives the same answer.
    pub(super) fn find_plane_obstacle_at(
        &self,
        assets: &LevelAssets,
        layer: u16,
        pos: MapPoint,
    ) -> Option<u16> {
        self.find_plane_obstacle_split(assets, layer, pos, pos)
    }

    /// Asymmetric variant used by the second-emergency probe in
    /// `cross_elevation_line`.  The bounding-box check is evaluated
    /// at the 2-units-ahead probe but the polygon containment check
    /// is at the actor's *current* map position.  In a band where the
    /// probe has left the current polygon but the actor has not, the
    /// old polygon is accepted.  Use `bbox_at` = probe and
    /// `polygon_at` = current map position to capture that.
    fn find_plane_obstacle_split(
        &self,
        assets: &LevelAssets,
        layer: u16,
        bbox_at: MapPoint,
        polygon_at: MapPoint,
    ) -> Option<u16> {
        for (oi, obs) in self.sight_obstacles(assets).iter_indexed() {
            if !obs.is_projection_area() {
                continue;
            }
            if obs.layer != layer {
                continue;
            }
            if !obs.box_screen.contains_point(bbox_at.to_geo()) {
                continue;
            }
            if !obs.contains_point_screen(polygon_at.to_geo()) {
                continue;
            }
            return Some(oi as u16);
        }
        None
    }

    fn crossed_elevation_obstacle(
        current: Option<u16>,
        left: Option<u16>,
        right: Option<u16>,
    ) -> Option<Option<u16>> {
        if current == left {
            Some(right)
        } else if current == right {
            Some(left)
        } else {
            None
        }
    }

    fn expand_move_box_for_command_extraction(bbox: crate::geo2d::BBox2D) -> crate::geo2d::BBox2D {
        if bbox.is_somewhere() {
            crate::geo2d::BBox2D::from_coords(
                bbox.x_min() - 0.5,
                bbox.y_min() - 0.5,
                bbox.x_max() + 0.5,
                bbox.y_max() + 0.5,
            )
        } else {
            bbox
        }
    }

    /// Swap an actor's sight-obstacle pointer to the opposite side of
    /// an elevation line it just crossed and update the footstep
    /// material + 3D plane projection.
    ///
    /// Given the line's stored left/right obstacle indices, flip the
    /// actor's `obstacle_index` to the other side.  The new obstacle
    /// is then routed through `set_obstacle_and_material` so the
    /// actor picks up the new top-plane and footstep material
    /// immediately.  Finally the sprite is reprojected from the new
    /// map position onto the new plane.
    ///
    /// When the actor's current obstacle matches neither side
    /// ("illegal bond crossing"), two emergency fallbacks run: walk
    /// the plane-sector registry at the actor's position for a
    /// containing plane, then if that misses retry at `pos + 2 *
    /// increment_map`.  Both are reproduced via
    /// [`Self::find_plane_obstacle_at`].
    ///
    /// `new_pos` is the actor's post-move map position. `increment_map`
    /// is a unit vector in the movement direction (used by the second
    /// emergency probe).
    pub(super) fn cross_elevation_line(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        line_idx: crate::fast_find_grid::LineIndex,
        new_pos: MapPoint,
        increment_map: crate::geo2d::Vec2D,
    ) {
        let line = match self.fast_grid.level.lines.get(usize::from(line_idx)) {
            Some(l) if l.is_elevation => l,
            _ => return,
        };
        let left = line.left_obstacle_index;
        let right = line.right_obstacle_index;

        let (current, layer) = match self
            .entities
            .get(entity_id.0 as usize)
            .and_then(|s| s.as_ref())
        {
            Some(e) => (
                e.element_data().obstacle_index().map(u16::from),
                e.element_data().layer(),
            ),
            None => return,
        };

        let mut next: Option<u16>;
        let mut found = true;

        if let Some(crossed) = Self::crossed_elevation_obstacle(current, left, right) {
            // legacy implementation compares raw obstacle pointers here, so NULL is a
            // valid side of an elevation line and must cross to the
            // opposite side instead of falling into the emergency path.
            next = crossed;
        } else {
            // "VERBOTEN: Illegal bond crossing" — current obstacle
            // matches neither side.  Walk projection-area obstacles
            // for one containing the actor's current position on its
            // layer.
            tracing::debug!(
                entity = ?entity_id,
                ?current,
                ?left,
                ?right,
                "cross_elevation_line: obstacle pointer doesn't match either side (illegal bond crossing)"
            );
            next = self.find_plane_obstacle_at(assets, layer, new_pos);
            if next.is_none() {
                // "STRENG VERBOTEN" — second emergency: probe two
                // map units ahead in the movement direction.  Gated
                // on a real direction (non-zero `increment_map`) —
                // when `check_for_line_crossing` early-returns on a
                // zero-length step the probe never reaches us, but
                // if a future caller wires this with an unfilled
                // increment we skip the second emergency rather than
                // probing in the wrong direction.
                let increment_computed =
                    increment_map.x.abs() > 1e-9 || increment_map.y.abs() > 1e-9;
                if increment_computed {
                    let probe = MapPoint::new(
                        new_pos.x + 2.0 * increment_map.x,
                        new_pos.y + 2.0 * increment_map.y,
                    );
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: second emergency, probing 2 units ahead at ({:.1}, {:.1})",
                        probe.x,
                        probe.y,
                    );
                    // Asymmetric predicate: bbox at the probe point,
                    // polygon containment at the actor's current
                    // (post-move) position.
                    next = self.find_plane_obstacle_split(assets, layer, probe, new_pos);
                }
                if next.is_none() {
                    // "ABSOLUT VERBOTEN" — give up; leave the actor's
                    // obstacle alone and skip the reprojection.
                    tracing::debug!(
                        entity = ?entity_id,
                        "cross_elevation_line: no projection area found at ({:.1}, {:.1})",
                        new_pos.x,
                        new_pos.y,
                    );
                    found = false;
                }
            }
        }

        if !found {
            return;
        }

        // Apply the new obstacle: updates element_data.obstacle_index,
        // element_data.material (footstep sounds), and the actor's
        // PositionInterface (obstacle, top-plane, material).
        self.set_obstacle_and_material(assets, entity_id, next);

        // Reproject the sprite onto the new plane.  Per-frame
        // movement updates `element_data.position_map` directly
        // without touching `position_iface`, so seed
        // `position_iface.position_map` from the freshly moved
        // position before recomputing 3D.
        if let Some(entity) = self.get_entity_mut(entity_id) {
            let pi = entity.position_iface_mut();
            pi.set_map_position(crate::coordinates::MapPoint {
                x: new_pos.x,
                y: new_pos.y,
            });
        }
    }

    /// Per-tick line-crossing dispatch for a moving actor.
    ///
    /// Restricted to elevation-line crossings.  For each elevation
    /// line the actor's `(old_pos, new_pos)` segment crosses on its
    /// current layer, we swap the actor's obstacle pointer via
    /// `cross_elevation_line`.  When multiple elevation lines are
    /// crossed in one tick, they are bubble-sorted by obstacle
    /// continuity so consecutive `cross_elevation_line` calls walk an
    /// actual chain of adjacent obstacles.
    ///
    /// Returns `true` if any elevation line was crossed — callers can
    /// use that to fire the human-specific `UpdateRoll` follow-up.
    pub(super) fn check_for_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) -> bool {
        // Early-out: exact same position means no crossing at all.
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return false;
        }

        let mut indices = self.fast_grid.get_crossing_elevation_line_indices(
            layer,
            old_pos.to_geo(),
            new_pos.to_geo(),
        );
        if indices.is_empty() {
            return false;
        }

        // Read the actor's current obstacle — used as the seed for the
        // sort when multiple lines are crossed.
        let mut current_obstacle = match self
            .entities
            .get(entity_id.0 as usize)
            .and_then(|s| s.as_ref())
        {
            Some(e) => e.element_data().obstacle_index().map(u16::from),
            None => return false,
        };

        // Bubble-sort elevation lines by obstacle continuity.  Each
        // iteration picks the next line whose left or right side
        // matches the running `current_obstacle`, swaps it into
        // place, and advances the running obstacle.  If no line
        // matches we stop sorting — later indices will still be
        // dispatched in whatever order they came out of the grid.
        let n = indices.len();
        if n > 1 {
            for i in 0..n.saturating_sub(1) {
                let mut matched = false;
                for j in i..n {
                    let line = match self.fast_grid.level.lines.get(usize::from(indices[j])) {
                        Some(l) => l,
                        None => continue,
                    };
                    if line.left_obstacle_index == current_obstacle {
                        current_obstacle = line.right_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                    if line.right_obstacle_index == current_obstacle {
                        current_obstacle = line.left_obstacle_index;
                        indices.swap(i, j);
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    break;
                }
            }
        }

        // Compute the unit movement vector for the second-emergency
        // probe inside `cross_elevation_line`.
        let increment_map = {
            let dx = new_pos.x - old_pos.x;
            let dy = new_pos.y - old_pos.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1e-6 {
                crate::geo2d::pt(dx / len, dy / len)
            } else {
                crate::geo2d::pt(0.0, 0.0)
            }
        };

        // Dispatch the swaps in order.
        for &idx in &indices {
            self.cross_elevation_line(assets, entity_id, idx, new_pos, increment_map);
        }

        true
    }

    /// Per-tick `LINE_PATCH` crossing dispatch for a PC.
    ///
    /// On crossing a LINE_PATCH line:
    ///
    /// ```text
    ///   if patch is active:
    ///       if patch.apply_sector contains GetPositionMap():
    ///           patch.Enter(actor)
    ///           if !patch.is_applied: patch.Apply()
    ///       else:
    ///           patch.Leave(actor)
    ///           if patch.is_applied && patch.any_occupant().is_none():
    ///               patch.Apply()
    /// ```
    ///
    /// Uses the PC's `new_pos` as the post-move probe.  `inside` means
    /// the PC just entered the apply polygon, `outside` means the PC
    /// just left it.  Patch state machine, FX entity, sight obstacles,
    /// grid sectors, and door rights are updated via
    /// `process_patch_effects`.
    pub(super) fn check_for_patch_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return;
        }

        let indices = self.fast_grid.get_crossing_patch_line_indices(
            layer,
            old_pos.to_geo(),
            new_pos.to_geo(),
        );
        if indices.is_empty() {
            return;
        }

        let occupant = crate::patch::OccupantId(entity_id.0);

        // `Patch::enter` / `leave` recurse onto the actor's carried
        // entity when the actor is a PC and is currently carrying
        // someone.  Resolve that here once (same entity for every
        // crossed patch this tick) so each per-patch Enter/Leave can
        // mirror the recursion.  `patch_cross_checks` only collects
        // PCs.
        let carried_occupant = self
            .get_entity(entity_id)
            .and_then(|e| match e {
                crate::element::Entity::Pc(pc) => pc.pc.carried,
                _ => None,
            })
            .map(|cid| crate::patch::OccupantId(cid.0));

        // Collect unique patches crossed this frame — one PC step can
        // intersect multiple boundary edges of the same apply polygon
        // (e.g. clipping a corner), and only the net Enter/Leave
        // decision matters, which is independent of edge count.
        let mut seen: Vec<crate::patch::PatchIndex> = Vec::new();
        for &line_idx in &indices {
            let Some(line) = self.fast_grid.level.lines.get(usize::from(line_idx)) else {
                continue;
            };
            let Some(patch_index) = line.patch_index else {
                continue;
            };
            if seen.contains(&patch_index) {
                continue;
            }
            seen.push(patch_index);
        }

        for patch_index in seen {
            // Snapshot the apply-sector polygon test result + active
            // state + applied state + occupant emptiness *before*
            // mutating the patch, so the action decision keeps the
            // strict order: is_active → is_inside → Enter/Leave →
            // conditional Apply.
            let patch_usize = patch_index.get() as usize;

            let (is_active, apply_sector_idx) = {
                let Some(game_host) = self.mission_script.as_ref().and_then(|s| s.game_host())
                else {
                    return;
                };
                let Some(patch) = game_host.patches.get(patch_usize) else {
                    continue;
                };
                (patch.is_active(), patch.apply_sector_index)
            };
            if !is_active {
                continue;
            }
            let apply_sector_idx = match apply_sector_idx {
                Some(i) => i,
                None => {
                    // No apply polygon declared — treat as "always
                    // outside" safety net and log once.
                    tracing::warn!(
                        patch = %patch_index,
                        "LINE_PATCH crossing on patch with no apply sector — skipping",
                    );
                    continue;
                }
            };
            let Some(apply_sector) = self.fast_grid.level.sectors.get(apply_sector_idx as usize)
            else {
                continue;
            };
            let inside_apply = apply_sector.contains_point(new_pos.to_geo());

            let effects = {
                let Some(game_host) = self.mission_script.as_mut().and_then(|s| s.game_host_mut())
                else {
                    return;
                };
                let Some(patch) = game_host.patches.get_mut(patch_usize) else {
                    continue;
                };
                if inside_apply {
                    // Entering the apply region.  `patch.enter`
                    // already logs and bails when the occupant is
                    // already in the list, so repeated single-tick
                    // crossings don't double-insert.
                    patch.enter(occupant);
                    // Carried-actor recursion: runs unconditionally
                    // after the warn/insert branch.
                    if let Some(carried) = carried_occupant {
                        patch.enter(carried);
                    }
                    if !patch.is_applied() {
                        patch.apply()
                    } else {
                        Vec::new()
                    }
                } else {
                    patch.leave(occupant);
                    // Carried-actor recursion: runs unconditionally
                    // after the find/remove branch.
                    if let Some(carried) = carried_occupant {
                        patch.leave(carried);
                    }
                    if patch.is_applied() && patch.any_occupant().is_none() {
                        patch.apply()
                    } else {
                        Vec::new()
                    }
                }
            };

            if !effects.is_empty() {
                self.process_patch_effects(assets, patch_index, effects);
            }
        }
    }

    /// Per-tick LINE_SOUND crossing dispatch for a moving actor.
    ///
    /// When the actor's `(old_pos, new_pos)` segment crosses one or
    /// more active LINE_SOUND grid lines on its current layer,
    /// refresh `actor.material` from the new ground material via
    /// [`MaterialSectors::material_at`] (which combines the
    /// "is-inside material polygon" test with the obstacle /
    /// default-material fallback in a single call).
    ///
    /// Updates both `ElementData::material` (read by footstep sound
    /// playback) and the actor's `PositionInterface` material so
    /// subsequent reads match.
    pub(super) fn check_for_sound_line_crossing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        old_pos: MapPoint,
        new_pos: MapPoint,
        layer: u16,
    ) {
        if (old_pos.x - new_pos.x).abs() < 1e-4 && (old_pos.y - new_pos.y).abs() < 1e-4 {
            return;
        }

        let indices = self.fast_grid.get_crossing_sound_line_indices(
            layer,
            old_pos.to_geo(),
            new_pos.to_geo(),
        );
        if indices.is_empty() {
            return;
        }

        // Resolve the new material once at the actor's just-updated
        // position.  Run the polygon containment test against the
        // line's source sector and fall back to the obstacle's
        // material when outside, then to the grid's default material
        // when no obstacle is set.  `MaterialSectors::material_at`
        // collapses the polygon scan + default-material fallback;
        // the obstacle-based fallback is then applied on top when no
        // sound polygon matches and the actor has an active obstacle.
        let polygon_material = assets.material_sectors.material_at(new_pos.to_geo());
        let new_material = if polygon_material == assets.material_sectors.default_material {
            // No SECTOR_SOUND polygon matched; apply the obstacle
            // fallback.  Without an obstacle pointer the default-
            // material returned by `material_at` is left in place.
            let obstacle_handle = self
                .get_entity(entity_id)
                .and_then(|e| e.element_data().obstacle_index());
            match obstacle_handle {
                Some(handle) => {
                    let idx: usize = handle.into();
                    self.sight_obstacles(assets)
                        .get(idx)
                        .map(|obs| crate::element::GameMaterial::from_u32(obs.material as u32))
                        .unwrap_or(polygon_material)
                }
                None => polygon_material,
            }
        } else {
            polygon_material
        };

        if let Some(entity) = self.get_entity_mut(entity_id) {
            let prev = entity.element_data().material();
            if prev != new_material {
                entity.element_data_mut().set_material(new_material);
                let pi = entity.position_iface_mut();
                pi.set_material(new_material);
                tracing::trace!(
                    ?entity_id,
                    ?prev,
                    ?new_material,
                    crossings = indices.len(),
                    "check_for_sound_line_crossing: refreshed material"
                );
            }
        }
    }

    /// Attempt to pathfind for a Move / Seek sequence element and, on
    /// success, populate its order queue, splice in startup / end
    /// transitions, set the actor's `ActionState`, and mark the element
    /// `InProgress`.
    ///
    /// Invoked once per element launch from the hourglass Move
    /// dispatch; on failure the caller pushes a
    /// [`FailedPathRequest`] for the 100-frame timeout and leaves the
    /// element `InProgress` with an empty order queue.  The pathfind
    /// is **not** re-attempted during that window — failed requests
    /// are never re-dispatched.
    pub(crate) fn try_dispatch_move_path(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        dest: MapPoint,
        mut move_action: OrderType,
    ) -> MovePathOutcome {
        use crate::engine::tick::apply_drunken_path_deviation;

        // Swap walking/running into the sword variant when the actor
        // is already in a sword action state — but only under two
        // gates:
        //   1. The post-transition posture is Upright — the swap is
        //      skipped for non-upright post-transition postures (e.g.
        //      CarryingCorpse, HelpingToClimb, ...).
        //   2. The action-state-after-transition is a sword state.
        // Read both from the SequenceElement rather than the live
        // entity state so a Move queued with a post-transition sword
        // state (e.g. launched from a posture/action transition that
        // hasn't applied yet) uses the intended post-transition
        // values.
        //
        // PC-fallback: when the sword variant isn't in the actor's
        // sprite profile (PCs like "Robin des bois" lack the
        // WalkingWithSword / RunningWithSword rows), leave the order
        // action as the upright variant.  The action state still
        // becomes MovingSword below so the per-frame combat
        // directional picker can substitute WalkingBackwardsSword /
        // Strafing*Sword.
        let (posture_after, mut action_after) = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| (e.posture_after_transition, e.action_state_after_transition))
            .unwrap_or_default();
        let elem_flags = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_fast = elem_flags.contains(crate::sequence::MoveFlags::FAST);
        let force_sword_movement = elem_flags.intersects(
            crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT
                | crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT,
        );
        if force_sword_movement && posture_after == crate::element::Posture::Upright {
            action_after = if is_fast || matches!(move_action, OrderType::RunningUpright) {
                crate::element::ActionState::MovingFastSword
            } else {
                crate::element::ActionState::MovingSword
            };
            if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx) {
                elem.action_state_after_transition = action_after;
            }
        }
        let sword_movement_context = (posture_after == crate::element::Posture::Upright
            && action_after.is_sword())
            || force_sword_movement;
        if sword_movement_context {
            let sword_variant = match move_action {
                // WALKING_UPRIGHT / WALKING_WITH_CORPSE →
                // WALKING_WITH_SWORD; RUNNING_UPRIGHT →
                // RUNNING_WITH_SWORD.
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    Some(OrderType::WalkingWithSword)
                }
                OrderType::RunningUpright => Some(OrderType::RunningWithSword),
                _ => None,
            };
            if let Some(want) = sword_variant
                && let Some(Some(entity)) = self.entities.get(owner.0 as usize)
                && entity.sprite().has_animation(want)
            {
                move_action = want;
            }
        }
        // PC shield-action arm: a shield-wielding PC on Upright
        // ground rewrites the movement element's stored `action`:
        //   WALKING_UPRIGHT / WALKING_WITH_CORPSE → WALKING_WITH_SHIELD
        //   WALKING_WITH_SHIELD                     → already set, no-op
        //   RUNNING_UPRIGHT                         → no
        //                                             running-with-shield
        //                                             anim, leave the
        //                                             upright variant
        //   default                                 → warn (would
        //                                             assert in dev).
        // Gated on PC because soldier/civilian shield holders fall
        // through to the upright animation.  Skip when the sprite
        // lacks the shield row (mirrors the sword PC fallback above).
        let owner_is_pc = self
            .entities
            .get(owner.0 as usize)
            .and_then(|s| s.as_ref())
            .is_some_and(|e| e.is_pc());
        if owner_is_pc
            && posture_after == crate::element::Posture::Upright
            && action_after.is_shield()
        {
            let want = match move_action {
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    Some(OrderType::WalkingWithShield)
                }
                OrderType::WalkingWithShield => None,
                OrderType::RunningUpright => None,
                _ => {
                    tracing::warn!(
                        ?owner,
                        ?move_action,
                        "DetermineMovementAnimation: shield action_state with \
                         unrecognised movement action",
                    );
                    None
                }
            };
            if let Some(want) = want
                && let Some(Some(entity)) = self.entities.get(owner.0 as usize)
                && entity.sprite().has_animation(want)
            {
                move_action = want;
            }
        }
        // Posture forces: non-Upright postures rewrite the action
        // regardless of the action-state inner switch.
        // CARRYING_CORPSE and CROUCHED are pure rewrites;
        // CARRYING_ON_SHOULDERS additionally sets `MoveFlags::REVERSED`
        // on the element flags.  The corpse-lost guard
        // (`WalkingWithCorpse → WalkingUpright`) closes the case where
        // a postponed Move retained `WalkingWithCorpse` after the
        // corpse target was lost — apply that under the Upright arm.
        let mut want_reverse_flag = false;
        match posture_after {
            crate::element::Posture::CarryingCorpse => {
                if let Some(Some(entity)) = self.entities.get(owner.0 as usize)
                    && entity.sprite().has_animation(OrderType::WalkingWithCorpse)
                {
                    move_action = OrderType::WalkingWithCorpse;
                }
            }
            crate::element::Posture::Crouched => {
                if let Some(Some(entity)) = self.entities.get(owner.0 as usize)
                    && entity.sprite().has_animation(OrderType::WalkingCrouched)
                {
                    move_action = OrderType::WalkingCrouched;
                }
            }
            crate::element::Posture::CarryingOnShoulders => {
                if let Some(Some(entity)) = self.entities.get(owner.0 as usize)
                    && entity
                        .sprite()
                        .has_animation(OrderType::WalkingCarryingOnShoulders)
                {
                    move_action = OrderType::WalkingCarryingOnShoulders;
                }
                want_reverse_flag = true;
            }
            crate::element::Posture::Upright => {
                // Inner action-state switch (non-lift Upright): for
                // action states in {Waiting, Bored, Moving,
                // MovingFast, *Bow*, Sleeping, Listening}, normalise
                // STAIRS / CLIMBING_* / CARRYING_ON_SHOULDERS /
                // CROUCHED inbound actions to WalkingUpright or
                // RunningUpright per `is_fast`.  WALKING_STAIRS always
                // normalises to WALKING_UPRIGHT regardless of speed.
                // The PC sword-variant guard is already handled by
                // the upstream sword-variant block + sprite-fallback
                // above.
                let owner_sector = self
                    .entities
                    .get(owner.0 as usize)
                    .and_then(|s| s.as_ref())
                    .and_then(|e| e.element_data().sector());
                let on_lift = self.sector_is_lift(owner_sector);
                let inner_arm = matches!(
                    action_after,
                    crate::element::ActionState::Waiting
                        | crate::element::ActionState::Bored
                        | crate::element::ActionState::Moving
                        | crate::element::ActionState::MovingFast
                        | crate::element::ActionState::Sleeping
                        | crate::element::ActionState::Listening
                ) || action_after.is_bow();
                if !on_lift && inner_arm {
                    let walk_or_run = if is_fast {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    };
                    move_action = match move_action {
                        // Pass-through.
                        OrderType::WalkingUpright
                        | OrderType::RunningUpright
                        | OrderType::RiderCharging => move_action,
                        // Stairs always → walking upright.
                        OrderType::WalkingStairs => OrderType::WalkingUpright,
                        // Climbing / carry-on-shoulders → walk/run upright.
                        OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingLadderUp
                        | OrderType::ClimbingLadderDown
                        | OrderType::ClimbingLadderUpFast
                        | OrderType::ClimbingLadderDownFast
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                        | OrderType::WalkingCarryingOnShoulders => walk_or_run,
                        // Crouched → walk/run upright.
                        OrderType::WalkingCrouched => walk_or_run,
                        // Default arm: leave `move_action` as-is for
                        // any non-listed type (sword/shield variants
                        // are already covered by the upstream blocks).
                        other => other,
                    };
                }
                // Corpse-lost guard.
                if move_action == OrderType::WalkingWithCorpse {
                    move_action = OrderType::WalkingUpright;
                }
            }
            _ => {}
        }
        move_action =
            self.determine_lift_movement_animation(owner, posture_after, move_action, dest);
        // Write the rewritten action back onto the movement sequence
        // element so downstream consumers (refresh-seek, post-process,
        // NPC AI re-reads) see it.  Apply both the action rewrite and
        // the CARRYING_ON_SHOULDERS REVERSED-flag mutation here.
        if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx)
            && let crate::sequence::SequenceElementData::Movement { flags, action, .. } =
                &mut elem.data
        {
            *action = move_action;
            if want_reverse_flag {
                *flags |= crate::sequence::MoveFlags::REVERSED;
            }
            if elem.posture_after_transition == crate::element::Posture::Undefined
                && let Some(Some(entity)) = self.entities.get(owner.0 as usize)
            {
                elem.posture_after_transition = entity.element_data().posture;
            }
        }

        // Read entity position / layer / sector / pathfinder index +
        // current move box + half diagonal (half diagonal drives the
        // thick-reachability pre-check below).
        let (
            mut source,
            entity_layer,
            entity_sector,
            pf_idx,
            mut move_box_map,
            half_diagonal,
            actor_passing_door,
        ) = {
            let entity = match self.entities.get(owner.0 as usize) {
                Some(Some(e)) => e,
                _ => return MovePathOutcome::ActorGone,
            };
            let elem = entity.element_data();
            let pi = entity.position_iface();
            let pf_idx = {
                let i = pi.get_pathfinder_index();
                if i == u16::MAX { 0 } else { i }
            };
            (
                geo2d::pt(elem.position_map().x, elem.position_map().y),
                elem.layer(),
                elem.sector().map(u16::from).unwrap_or(0),
                pf_idx,
                *pi.get_move_box_map(),
                pi.get_half_diagonal(),
                entity
                    .actor_data()
                    .is_some_and(|actor| actor.active_door_pass.is_some()),
            )
        };

        // legacy implementation RHElementActor::InstructOwner(RHCOMMAND_MOVE) first tries
        // to extract an unauthorized actor from motion obstacles by
        // expanding GetMoveBoxMap() by 0.5 on every side, snapping the
        // actor to the recovered box center, and recomputing position.
        // The later RHPathFinder::AddPathRequest extraction still runs
        // on the unexpanded move box when pathfinding is needed.
        if !self
            .fast_grid
            .is_position_authorized(&move_box_map, entity_layer)
        {
            let mut box_element = Self::expand_move_box_for_command_extraction(move_box_map);
            if self
                .fast_grid
                .find_authorized_position(&mut box_element, entity_layer)
            {
                let center = box_element.center();
                source = geo2d::pt(center.x, center.y);
                if let Some(entity) = self.get_entity_mut(owner) {
                    entity
                        .position_iface_mut()
                        .set_map_position(crate::coordinates::MapPoint::from_geo(source));
                    let elem = entity.element_data_mut();
                    elem.set_position_map(crate::coordinates::MapPoint {
                        x: source.x,
                        y: source.y,
                    });
                    elem.update_grid_cell();
                    move_box_map = *entity.position_iface().get_move_box_map();
                }
            }
        }

        // Before queuing a path request, if the move is flagged
        // MAP / STRAIGHT, or the source→dest segment is
        // thick-reachable, skip the pathfinder entirely and emit a
        // single direct order.  The pathfinder is never invoked when
        // a straight line suffices.
        //
        // Without this pre-check, short clicks that are directly
        // walkable still hit A*, which can route the actor through
        // source-adjacent graph nodes (extra waypoints around
        // `PassAroundLastNode`) and produce the "keeps moving old
        // direction briefly" click-walk regression.
        let move_flags = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_pass_door = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| e.command == crate::element::Command::PassDoor);
        let source_is_lift_rail = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(entity_sector as i16))
            .and_then(|gs| gs.lift_type)
            .is_some_and(|lt| {
                matches!(
                    lt,
                    crate::sector::LiftType::Wall | crate::sector::LiftType::Ladder
                )
            });
        let straight_ok = move_flags.contains(crate::sequence::MoveFlags::MAP)
            || move_flags.contains(crate::sequence::MoveFlags::STRAIGHT)
            || is_pass_door
            || actor_passing_door
            || source_is_lift_rail
            || self
                .fast_grid
                .is_reachable_thick(source.into(), dest, entity_layer, half_diagonal);

        // Before submitting a path request, check whether the actor's
        // move box is in an authorized position.  This mirrors legacy implementation
        // `RHPathFinder::AddPathRequest`; direct MAP / STRAIGHT /
        // thick-reachable moves do not enter the pathfinder and do
        // not run this extraction gate.
        //
        // If extraction is needed, call `find_authorized_position` to
        // mutate the box to a nearby valid spot, set
        // `use_first_point = true`, and snap the request source to the
        // recovered box centre.  When extraction fails, stop the actor
        // and `Wait` it.
        //
        // Without this snap the downstream strict source-authorization
        // check rejects every candidate and the actor is permanently
        // stuck — A* can't seed.  An earlier fallback only handles
        // the inverse case (source authorized but corridor too thin);
        // this handles "actor must always stay on an authorized
        // position" by pre-snapping the request source.
        let mut use_first_point = false;
        let skip_source_extraction = is_pass_door || actor_passing_door || source_is_lift_rail;
        if !straight_ok
            && !skip_source_extraction
            && !self
                .fast_grid
                .is_position_authorized(&move_box_map, entity_layer)
        {
            let mut box_element = move_box_map;
            if !self
                .fast_grid
                .find_authorized_position(&mut box_element, entity_layer)
            {
                // Extraction failed; stop the actor and bail.  Route
                // through `stop_owner` (which clears active sequences
                // and pending path requests for this owner) and
                // launch a `Wait` sequence element at `Wait` priority.
                tracing::debug!(
                    actor = ?owner,
                    src_x = source.x,
                    src_y = source.y,
                    layer = entity_layer,
                    "try_dispatch_move_path: actor cannot be extracted from obstacle (Stop + Wait)",
                );
                self.stop_owner(owner, crate::sequence::SequencePriority::Wait);
                let mut wait_elem = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::Wait,
                    Some(owner),
                );
                wait_elem.priority = crate::sequence::SequencePriority::Wait;
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(wait_elem);
                self.launch_sequence(seq);
                return MovePathOutcome::Failed;
            }
            let center = box_element.center();
            tracing::info!(
                actor = ?owner,
                old_src_x = source.x,
                old_src_y = source.y,
                new_src_x = center.x,
                new_src_y = center.y,
                "try_dispatch_move_path: extracted source from obstacle (use_first_point=true)",
            );
            source = geo2d::pt(center.x, center.y);
            use_first_point = true;
        }

        // Run pathfinder — unless the straight-line pre-check above
        // said a direct order suffices, in which case we skip A* and
        // build a two-point "path" that the downstream emission loop
        // turns into a single walking order to `dest`.
        let mut waypoints = if straight_ok {
            vec![MapPoint::from_geo(source), dest]
        } else {
            let path = self.pathfinder.find_path(
                assets.pathfinder_graph.as_ref(),
                &self.fast_grid,
                entity_layer,
                entity_sector,
                pf_idx,
                MapPoint::from_geo(source),
                dest,
                use_first_point,
            );
            match path {
                Some(w) => w,
                None => {
                    tracing::debug!(
                        actor = ?owner,
                        ?seq_id,
                        elem_idx,
                        src_x = source.x,
                        src_y = source.y,
                        dst_x = dest.x,
                        dst_y = dest.y,
                        layer = entity_layer,
                        sector = entity_sector,
                        is_pass_door,
                        actor_passing_door,
                        source_is_lift_rail,
                        ?move_flags,
                        "try_dispatch_move_path: pathfind FAILED",
                    );
                    return MovePathOutcome::Failed;
                }
            }
        };

        // Drunken-soldier path deviation.  Only applies to upright
        // walking/running animations and not to PassDoor commands.
        let is_movement_anim = matches!(
            move_action,
            OrderType::WalkingUpright | OrderType::RunningUpright
        );
        if is_movement_anim && !is_pass_door {
            let blood_alcohol = self
                .entities
                .get(owner.0 as usize)
                .and_then(|s| s.as_ref())
                .and_then(|e| e.npc_data())
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.blood_alcohol)
                .unwrap_or(0);
            if blood_alcohol > 0 {
                let (half_diag, move_box) = self
                    .entities
                    .get(owner.0 as usize)
                    .and_then(|s| s.as_ref())
                    .map(|e| e.position_iface())
                    .map(|pi| (pi.get_half_diagonal(), *pi.get_move_box()))
                    .unwrap_or_default();
                let geo_waypoints: Vec<crate::geo2d::GeoPoint2D> =
                    waypoints.iter().copied().map(MapPoint::to_geo).collect();
                waypoints = apply_drunken_path_deviation(
                    geo_waypoints,
                    source,
                    blood_alcohol,
                    move_action == OrderType::RunningUpright,
                    entity_layer,
                    &move_box,
                    half_diag,
                    &self.fast_grid,
                    &mut self.rng,
                )
                .into_iter()
                .map(MapPoint::from_geo)
                .collect();
            }
        }

        tracing::trace!(
            actor = ?owner,
            ?seq_id,
            elem_idx,
            wp = waypoints.len(),
            ?move_action,
            ?elem_flags,
            sword_movement_context,
            "try_dispatch_move_path: dispatched {} waypoints to actor",
            waypoints.len(),
        );

        // Build one walking/running order per waypoint.  The final
        // order carries the element's tolerance + antagonist, and
        // every order carries the element's reverse flag.
        //
        // The `antagonist`: when SEEK+USE_POINT the target element is
        // *not* carried on the move (the seek is to a hotspot, not to
        // the antagonist itself); otherwise the movement element's
        // `element` (antagonist) rides along on the final order so
        // downstream consumers (touch-on-Done etc.) can resolve it.
        let (elem_tolerance, elem_flags, elem_antagonist) = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement {
                    tolerance,
                    flags,
                    element,
                    ..
                } => Some((*tolerance, *flags, *element)),
                _ => None,
            })
            .unwrap_or((0.0, crate::sequence::MoveFlags::empty(), None));
        let reverse = elem_flags.contains(crate::sequence::MoveFlags::REVERSED);
        let antagonist = if elem_flags.contains(crate::sequence::MoveFlags::SEEK)
            && elem_flags.contains(crate::sequence::MoveFlags::USE_POINT)
        {
            None
        } else {
            elem_antagonist
        };

        // `use_first_point` handling: the emission loop starts at
        // index 0 if set, otherwise 1.
        //
        // * `use_first_point == false` — the normal case where the
        //   source was already authorized.  `path[0]` IS the actor's
        //   current position (the pathfinder returns
        //   `[source, ..., goal]` for graph paths), so skip it to
        //   avoid a zero-length first order.  Direct paths return
        //   just `[goal]` (len == 1) and the skip doesn't apply.
        //
        // * `use_first_point == true` — set above when the source
        //   had to be extracted from an obstacle.  `path[0]` is the
        //   snapped source, NOT the actor's current position; keep
        //   it as the first waypoint so the actor walks back to safe
        //   ground before continuing.  (For direct paths this is a
        //   no-op: `[goal]` stays a single waypoint and the actor
        //   walks straight to goal — anti-collision handles the small
        //   obstacle clip on that first leg.)
        if !use_first_point && waypoints.len() > 1 {
            let first = waypoints[0];
            if (first.x - source.x).abs() < f32::EPSILON
                && (first.y - source.y).abs() < f32::EPSILON
            {
                waypoints.remove(0);
            }
        }
        {
            let next_order_id = &mut self.next_order_id;
            if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx) {
                crate::movement::build_orders_from_path(
                    elem,
                    &waypoints,
                    move_action,
                    elem_tolerance,
                    reverse,
                    antagonist,
                    next_order_id,
                );
            }
        }

        // Splice startup / end transitions into the order queue
        // based on the actor's posture + action state.
        let orders_before = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.orders.len())
            .unwrap_or(0);
        let first_order_before = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| e.orders.front())
            .map(|o| o.order_type);
        let had_launch_transition = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| e.num_transition_orders > 0 && e.orders.front().is_some());
        self.post_process_path(seq_id, elem_idx);
        let (orders_after, first_order_after) = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| (e.orders.len(), e.orders.front().map(|o| o.order_type)))
            .unwrap_or((orders_before, first_order_before));
        let have_start_transition = had_launch_transition
            // `post_process_path` can prepend walk/run startup transitions.
            // `generate_transition` can also have queued posture transitions
            // already, e.g. PC Spy/Tree uncloaking before a move.
            //
            // TODO: Consider tracking start-vs-end transition order spans
            // explicitly on SequenceElement instead of inferring from the
            // front order and insertion deltas.
            ||
            orders_after > orders_before && first_order_after != first_order_before;

        // For seek elements with an entity target, snapshot the
        // target's current map position onto the actor so the
        // per-tick `tick_refresh_seeks` scan can detect when the
        // target has moved > 10 units and re-route.
        let seek_snapshot: Option<(crate::coordinates::MapPoint, Option<EntityId>)> =
            if elem_flags.contains(crate::sequence::MoveFlags::SEEK) {
                elem_antagonist.and_then(|id| {
                    self.get_entity(id)
                        .map(|e| (e.element_data().position_map(), Some(id)))
                })
            } else {
                None
            };

        // Update actor state.  When a startup transition was prepended,
        // leave `action_state` alone so the transition's `MS::Done`
        // handler flips it to the moving state on completion.  End-only
        // transitions are appended behind the movement orders; they must
        // not delay the actor entering Moving/MovingSword now.
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            if !have_start_transition {
                actor.action_state = if sword_movement_context {
                    if is_fast
                        || matches!(
                            move_action,
                            OrderType::RunningUpright | OrderType::RunningWithSword
                        )
                    {
                        crate::element::ActionState::MovingFastSword
                    } else {
                        crate::element::ActionState::MovingSword
                    }
                } else {
                    match move_action {
                        OrderType::WalkingWithSword => crate::element::ActionState::MovingSword,
                        OrderType::RunningWithSword => crate::element::ActionState::MovingFastSword,
                        OrderType::RunningUpright => crate::element::ActionState::MovingFast,
                        _ => crate::element::ActionState::Moving,
                    }
                };
            }
            actor.active_movement = ActiveMovement::new(seq_id, elem_idx);
            if let Some((target_pos, target_id)) = seek_snapshot {
                actor.last_seek_target_position = target_pos;
                actor.seek_target = target_id;
                // TIME_SEEK_REFRESH = 25.
                actor.seek_refresh_wait = 25;
            }
            // `sequence_element_started` flips true once the movement
            // element promotes to InProgress.  Read by
            // `non_interruptable_guard` to gate the PASS_DOOR + MOVE
            // IMPOSSIBLE fast-fail.
            actor.sequence_element_started = true;
        }

        // Transition element to InProgress.
        self.sequence_manager.element_in_progress(seq_id, elem_idx);

        MovePathOutcome::Success
    }

    /// Age out [`FailedPathRequest`] entries.  Runs once per hourglass.
    ///
    /// When `first_fail_frame + 100 < frame_counter`, mark the
    /// element Impossible (and, for PCs, fire the
    /// `HERO_UNABLE_TO_DO_SOMETHING` speech line).
    ///
    /// No re-dispatch — failed requests are not re-submitted during
    /// the 100-frame window.  The element sits waiting and the
    /// pathfinder's queue is free to process unrelated requests.
    /// Any external state change that would unblock the actor is
    /// handled by the calling code (e.g. cancelling on halt /
    /// postpone, new Move element replacing this one).
    ///
    /// Entries whose owning element is no longer live (cancelled,
    /// cascaded to terminal state, index reused) are silently dropped.
    pub(super) fn process_failed_path_timeouts(&mut self, assets: &LevelAssets) {
        let now = self.frame_counter;
        let mut still_waiting = Vec::new();
        for req in std::mem::take(&mut self.failed_path_requests) {
            // Element cancelled / finished / reused — drop silently.
            let still_live = self
                .sequence_manager
                .get_element(req.seq_id, req.elem_idx)
                .map(|e| {
                    e.owner == Some(req.owner)
                        && matches!(e.state, crate::sequence::SequenceState::InProgress)
                        && matches!(
                            e.command,
                            crate::element::Command::Move | crate::element::Command::Seek
                        )
                })
                .unwrap_or(false);
            if !still_live {
                continue;
            }

            // Guard: strictly greater than 100 elapsed frames.
            if now.saturating_sub(req.first_fail_frame) <= 100 {
                still_waiting.push(req);
                continue;
            }

            let is_pc = self
                .entities
                .get(req.owner.0 as usize)
                .and_then(|s| s.as_ref())
                .map(|e| e.is_pc())
                .unwrap_or(false);
            if is_pc {
                self.hero_speaking(
                    assets,
                    req.owner,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }

            self.sequence_manager
                .element_impossible(req.seq_id, req.elem_idx);
            tracing::debug!(
                actor = ?req.owner,
                seq_id = ?req.seq_id,
                elem_idx = req.elem_idx,
                age = now.saturating_sub(req.first_fail_frame),
                "failed_path: 100-frame timeout expired — marking Impossible",
            );
        }
        self.failed_path_requests = still_waiting;
    }
}

#[cfg(test)]
mod line_jump_tests {
    use super::*;
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElementData};

    #[test]
    fn line_jump_click_sequence_moves_to_line_then_jumps_then_moves_to_click() {
        let owner = EntityId(7);
        let source_idx = crate::jump_line::JumpLineIndex::new(2).unwrap();
        let dest_idx = crate::jump_line::JumpLineIndex::new(3).unwrap();
        let mut source_line = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(10.0, 20.0),
            crate::coordinates::map_pt(30.0, 20.0),
            0.0,
            0.0,
        );
        source_line.layer = 4;

        let seq = build_line_jump_click_sequence(
            owner,
            OrderType::RunningUpright,
            source_idx,
            &source_line,
            dest_idx,
            crate::coordinates::map_pt(90.0, 120.0),
            5,
            1.0,
        );

        assert_eq!(seq.elements.len(), 3);
        assert_eq!(seq.elements[0].command, Command::Move);
        match &seq.elements[0].data {
            SequenceElementData::Movement {
                destination,
                layer,
                line_id,
                flags,
                ..
            } => {
                assert_eq!((destination.x, destination.y), (20.0, 20.0));
                assert_eq!(*layer, 4);
                assert_eq!(*line_id, Some(source_idx));
                assert!(flags.contains(MoveFlags::LINE));
                assert!(flags.contains(MoveFlags::TO_JUMP));
            }
            other => panic!("expected movement element, got {other:?}"),
        }

        assert_eq!(seq.elements[1].command, Command::Jump);
        assert!(matches!(
            seq.elements[1].get_property(Field::JumplineSource),
            Some(FieldValue::LineId(idx)) if *idx == source_idx
        ));
        assert!(matches!(
            seq.elements[1].get_property(Field::JumplineDestination),
            Some(FieldValue::LineId(idx)) if *idx == dest_idx
        ));

        assert_eq!(seq.elements[2].command, Command::Move);
        match &seq.elements[2].data {
            SequenceElementData::Movement {
                destination,
                layer,
                flags,
                line_id,
                ..
            } => {
                assert_eq!((destination.x, destination.y), (90.0, 120.0));
                assert_eq!(*layer, 5);
                assert!(flags.is_empty());
                assert_eq!(*line_id, None);
            }
            other => panic!("expected final movement element, got {other:?}"),
        }
    }

    #[test]
    fn force_sword_movement_marks_all_movement_elements() {
        let owner = EntityId(7);
        let source_idx = crate::jump_line::JumpLineIndex::new(2).unwrap();
        let dest_idx = crate::jump_line::JumpLineIndex::new(3).unwrap();
        let source_line = crate::jump_line::JumpLine::new(
            crate::coordinates::map_pt(10.0, 20.0),
            crate::coordinates::map_pt(30.0, 20.0),
            0.0,
            0.0,
        );

        let mut seq = build_line_jump_click_sequence(
            owner,
            OrderType::WalkingUpright,
            source_idx,
            &source_line,
            dest_idx,
            crate::coordinates::map_pt(90.0, 120.0),
            5,
            1.0,
        );

        force_sword_movement_for_sequence(&mut seq);

        let movement_flags: Vec<_> = seq
            .elements
            .iter()
            .filter_map(|elem| match &elem.data {
                SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .collect();

        assert_eq!(movement_flags.len(), 2);
        assert!(
            movement_flags
                .iter()
                .all(|flags| flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT))
        );
    }

    #[test]
    fn running_with_sword_uses_distance_motion() {
        assert!(order_uses_distance_motion(OrderType::RunningWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingWithSword));
        assert!(order_uses_distance_motion(OrderType::WalkingSword));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionRunningUprightWaitingUpright
        ));
        assert!(!order_uses_distance_motion(
            OrderType::TransitionSpecialWaitingUpright
        ));
    }

    #[test]
    fn movement_execute_state_effects_match_transition_execute() {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionSpecialWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWalkingUpright,
                MotionState::Terminated
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::WalkingUpright, MotionState::Start),
            Some((Posture::Upright, ActionState::Moving))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightBoredWaitingUpright,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionWaitingUprightWaitingUprightBored,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Bored))
        );
        assert_eq!(
            movement_execute_state_effect(OrderType::TransitionCrouchingDown, MotionState::Done),
            Some((Posture::Crouched, ActionState::Waiting))
        );
        assert_eq!(
            movement_execute_state_effect(
                OrderType::TransitionLeaningOutWaitingAlerted,
                MotionState::Done
            ),
            Some((Posture::Upright, ActionState::Waiting))
        );
    }

    #[test]
    fn only_explicit_in_place_movement_transitions_accept_zero_target() {
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSpecialWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightSpecial
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightBoredWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWaitingUprightBored
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingUp
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionCrouchingDown
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionSittingWaitingUpright
        ));
        assert!(is_in_place_movement_transition(
            OrderType::TransitionLeaningOutWaitingAlerted
        ));
        assert!(is_in_place_movement_transition(OrderType::StandingUp));
        assert!(is_in_place_movement_transition(OrderType::StandingUpSword));
        assert!(is_in_place_movement_transition(OrderType::StandingUpBow));
        assert!(is_in_place_movement_transition(OrderType::LoweringShield));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWaitingUprightWalkingUpright
        ));
        assert!(!is_in_place_movement_transition(
            OrderType::TransitionWalkingUprightWaitingUpright
        ));
    }

    #[test]
    fn elevation_crossing_matches_null_obstacle_side() {
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(None, None, Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(50), None, Some(50)),
            Some(None)
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(49), Some(49), Some(50)),
            Some(Some(50))
        );
        assert_eq!(
            EngineInner::crossed_elevation_obstacle(Some(99), Some(49), Some(50)),
            None
        );
    }

    #[test]
    fn command_extraction_expands_move_box_like_original() {
        let bbox = crate::geo2d::BBox2D::from_coords(10.0, 20.0, 30.0, 40.0);
        let expanded = EngineInner::expand_move_box_for_command_extraction(bbox);
        assert_eq!(expanded.x_min(), 9.5);
        assert_eq!(expanded.y_min(), 19.5);
        assert_eq!(expanded.x_max(), 30.5);
        assert_eq!(expanded.y_max(), 40.5);
    }
}
