//! Door-pass translation and execution.
//!
//! Translates a door-pass into a chain of walk/transition steps and
//! executes the layer/sector swap when the actor crosses the door.

use super::*;
use crate::coordinates::MapPoint;
use crate::element::{ActiveDoorPass, DoorPassStep, EntityId, Posture};
use crate::gate::DoorType;
use crate::order::OrderType;
use crate::sector::LiftType;
use std::collections::VecDeque;

// ─── Step construction helpers ──────────────────────────────────────

fn walk(dest: MapPoint, action: OrderType) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: false,
        compute_direction: true,
        tolerance: 0.0,
    }
}

fn walk_tol(dest: MapPoint, action: OrderType, tolerance: f32) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: false,
        compute_direction: true,
        tolerance,
    }
}

fn walk_rev_tol(dest: MapPoint, action: OrderType, tolerance: f32) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: true,
        compute_direction: true,
        tolerance,
    }
}

fn walk_nodir(dest: MapPoint, action: OrderType) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: false,
        compute_direction: false,
        tolerance: 0.0,
    }
}

fn walk_nodir_tol(dest: MapPoint, action: OrderType, tolerance: f32) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: false,
        compute_direction: false,
        tolerance,
    }
}

fn walk_rev_nodir(dest: MapPoint, action: OrderType) -> DoorPassStep {
    DoorPassStep::Walk {
        destination: dest,
        action,
        reverse: true,
        compute_direction: false,
        tolerance: 0.0,
    }
}

fn transition(action: OrderType) -> DoorPassStep {
    DoorPassStep::Transition {
        action,
        reverse: false,
    }
}

fn transition_rev(action: OrderType) -> DoorPassStep {
    DoorPassStep::Transition {
        action,
        reverse: true,
    }
}

const fn passing_door() -> DoorPassStep {
    DoorPassStep::PassingDoor
}

// ─── Context for building step chains ───────────────────────────────

/// All the data needed to build a door-pass step chain.
/// Extracted from the door, sector, and actor data before step construction.
struct DoorPassContext {
    door_type: DoorType,
    point_mid: MapPoint,
    point_in: MapPoint,
    point_out: MapPoint,
    direct: bool,
    is_pc: bool,
    is_soldier_attentive: bool,
    action: OrderType,
    is_carrying_on_shoulders: bool,
    sector_out_forces_crouch: bool,
    sector_in_forces_crouch: bool,
    is_high: bool,
    is_crenel: bool,
    /// First-walk tolerance for the high/direct ladder pass (climb DOWN).
    /// Reads `Sprite::distance_for_animation` for the upcoming climb-down
    /// transition (and crouching-down for non-soldier PCs).  See
    /// `build_door_pass` for the per-actor selection.
    tol_ladder_high_direct: f32,
    /// First-walk tolerance for the low/direct ladder pass (climb UP).
    /// Distance for `TransitionWaitingUprightClimbingLadderUp`.
    tol_ladder_low_direct: f32,
    /// First-walk tolerance for the high/direct wall pass, non-crenel
    /// (climb DOWN).  Sum of `TransitionCrouchingDown` and
    /// `TransitionWaitingCrouchedClimbingWallDown` animation distances.
    tol_wall_high_direct_noncrenel: f32,
    /// First-walk tolerance for the high/direct wall pass, crenel.
    /// Distance for `TransitionWaitingCrouchedClimbingWallDownCrenel`.
    tol_wall_high_direct_crenel: f32,
    /// First-walk tolerance for the low/direct wall pass (climb UP).
    /// Distance for `TransitionWaitingUprightClimbingWallUp`.
    tol_wall_low_direct: f32,
}

// ─── Building door translation ──────────────────────────────────────

fn translate_building(ctx: &DoorPassContext) -> VecDeque<DoorPassStep> {
    let mut s = VecDeque::new();
    let action = ctx.action;

    // PCs get a `Select` step between the walk-to-mid step and PASSING_DOOR;
    // the hulk fade speed comes from the remaining-leg distance * 0.03.
    let select_speed = |from: MapPoint, to: MapPoint| -> f32 {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        (dx * dx + dy * dy).sqrt() * 0.03
    };

    if ctx.direct {
        // Outside -> inside
        s.push_back(walk(ctx.point_mid, action));
        if ctx.is_pc {
            s.push_back(DoorPassStep::Select {
                speed: select_speed(ctx.point_mid, ctx.point_out),
            });
        }
        s.push_back(passing_door());
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_in,
            action,
            reverse: false,
            compute_direction: false,
            tolerance: 0.0,
        });
        // Building-trap: reverse ladder-down animation after entering
        if ctx.door_type == DoorType::BuildingTrap {
            s.push_back(walk_rev_nodir(ctx.point_in, OrderType::ClimbingLadderDown));
        }
        s.push_back(passing_door());
    } else {
        // Inside -> outside
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_mid,
            action,
            reverse: false,
            compute_direction: false,
            tolerance: 0.0,
        });
        if ctx.is_pc {
            s.push_back(DoorPassStep::Select {
                speed: select_speed(ctx.point_mid, ctx.point_in),
            });
        }
        s.push_back(passing_door());
        s.push_back(walk(ctx.point_out, action));
        s.push_back(passing_door());
    }
    s
}

// ─── Ladder door translation ────────────────────────────────────────

/// Walk-step tolerance applied to the walk-to-mid step of the
/// high/non-direct ladder pass so the climb-up transition lands the
/// actor at the ladder's rung rail instead of overshooting the exact
/// point.
const TELEPORT_LADDER: f32 = 45.0;

fn translate_ladder(ctx: &DoorPassContext) -> VecDeque<DoorPassStep> {
    let mut s = VecDeque::new();

    if ctx.is_high {
        if ctx.direct {
            // High, outside -> inside (climb DOWN the ladder).
            // The first walk-to-mid step has a tolerance equal to the
            // climb-down transition distance (plus crouching-down for
            // non-soldier PCs); precomputed in `build_door_pass` via
            // `Sprite::distance_for_animation`.
            s.push_back(walk_rev_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_ladder_high_direct,
            ));
            s.push_back(transition_rev(OrderType::Turning));
            if ctx.is_pc {
                s.push_back(transition_rev(OrderType::TransitionCrouchingDown));
            }
            let climb_start = if ctx.is_soldier_attentive {
                OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
            } else {
                OrderType::TransitionWaitingCrouchedClimbingLadderDown
            };
            s.push_back(walk_rev_nodir(ctx.point_mid, climb_start));
            s.push_back(passing_door());
            s.push_back(walk_rev_nodir(ctx.point_in, OrderType::ClimbingLadderDown));
        } else {
            // High, inside -> outside (climb UP the ladder).
            // `TELEPORT_LADDER` (45.0) is set as tolerance on the first
            // walk-to-mid step so the climb-up animation (which already
            // moves the actor past the midpoint) ends before the
            // waypoint is exactly reached.
            s.push_back(walk_nodir_tol(
                ctx.point_mid,
                OrderType::ClimbingLadderUp,
                TELEPORT_LADDER,
            ));
            let climb_end = if ctx.is_soldier_attentive {
                OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            } else {
                OrderType::TransitionClimbingLadderUpWaitingCrouched
            };
            s.push_back(walk_nodir(ctx.point_mid, climb_end));
            if ctx.is_pc && !ctx.sector_out_forces_crouch {
                s.push_back(transition(OrderType::TransitionCrouchingUp));
            }
            s.push_back(passing_door());
            let exit_action = if ctx.is_pc && ctx.sector_out_forces_crouch {
                OrderType::WalkingCrouched
            } else {
                OrderType::WalkingUpright
            };
            s.push_back(walk(ctx.point_out, exit_action));
            s.push_back(passing_door());
        }
    } else {
        if ctx.direct {
            // Low, outside -> inside (climb UP).  Tolerance is the
            // `TransitionWaitingUprightClimbingLadderUp` animation
            // distance, precomputed in `build_door_pass` and threaded
            // as `ctx.tol_ladder_low_direct`.
            s.push_back(walk_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_ladder_low_direct,
            ));
            let climb_start = if ctx.is_soldier_attentive {
                OrderType::TransitionWaitingUprightClimbingLadderUpAlerted
            } else {
                OrderType::TransitionWaitingUprightClimbingLadderUp
            };
            s.push_back(walk_nodir(ctx.point_mid, climb_start));
            s.push_back(passing_door());
            s.push_back(walk_nodir(ctx.point_in, OrderType::ClimbingLadderUp));
            s.push_back(passing_door());
        } else {
            // Low, inside -> outside (climb DOWN)
            s.push_back(walk_nodir(ctx.point_mid, OrderType::ClimbingLadderDown));
            let climb_end = if ctx.is_soldier_attentive {
                OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
            } else {
                OrderType::TransitionClimbingLadderDownWaitingUpright
            };
            s.push_back(transition(climb_end));
            s.push_back(passing_door());
            if ctx.is_pc && ctx.sector_out_forces_crouch {
                s.push_back(transition(OrderType::TransitionCrouchingDown));
                s.push_back(walk(ctx.point_out, OrderType::WalkingCrouched));
            } else {
                s.push_back(walk(ctx.point_out, OrderType::WalkingUpright));
            }
            s.push_back(passing_door());
        }
    }
    s
}

// ─── Wall door translation ──────────────────────────────────────────

/// Walk-step tolerance applied to the walk-to-mid step of the
/// high/non-direct wall pass (climb-up).
const TELEPORT_WALL: f32 = 60.0;

fn translate_wall(ctx: &DoorPassContext) -> VecDeque<DoorPassStep> {
    let mut s = VecDeque::new();

    if ctx.is_high {
        if ctx.direct {
            // High, outside -> inside (climb DOWN the wall).  The first
            // walk-to-mid tolerance comes from animation distances
            // (different for crenel vs non-crenel); precomputed in
            // `build_door_pass` and threaded via the two
            // `tol_wall_high_direct_*` ctx fields.
            if !ctx.is_crenel {
                s.push_back(walk_rev_tol(
                    ctx.point_mid,
                    OrderType::WalkingUpright,
                    ctx.tol_wall_high_direct_noncrenel,
                ));
                s.push_back(transition_rev(OrderType::Turning));
                if ctx.is_pc {
                    s.push_back(transition_rev(OrderType::TransitionCrouchingDown));
                }
                s.push_back(walk_rev_nodir(
                    ctx.point_mid,
                    OrderType::TransitionWaitingCrouchedClimbingWallDown,
                ));
            } else {
                // Crenel variant
                s.push_back(walk_tol(
                    ctx.point_mid,
                    OrderType::WalkingUpright,
                    ctx.tol_wall_high_direct_crenel,
                ));
                s.push_back(walk_nodir(
                    ctx.point_mid,
                    OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                ));
            }
            s.push_back(passing_door());
            s.push_back(walk_rev_nodir(ctx.point_in, OrderType::ClimbingWallDown));
            s.push_back(passing_door());
        } else {
            // High, inside -> outside (climb UP the wall).
            // `TELEPORT_WALL` (60.0) is set as tolerance on the first
            // walk-to-mid step so the climb-up ends before the waypoint
            // is exactly reached (the animation itself carries the
            // actor past the point).
            s.push_back(walk_nodir_tol(
                ctx.point_mid,
                OrderType::ClimbingWallUp,
                TELEPORT_WALL,
            ));
            if ctx.is_crenel {
                s.push_back(walk_nodir(
                    ctx.point_out,
                    OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
                ));
            } else {
                s.push_back(walk_nodir(
                    ctx.point_mid,
                    OrderType::TransitionClimbingWallUpWaitingCrouched,
                ));
            }
            s.push_back(passing_door());
            if ctx.is_pc && !ctx.sector_out_forces_crouch {
                s.push_back(transition(OrderType::TransitionCrouchingUp));
            }
            let exit_action = if !ctx.is_pc || !ctx.sector_out_forces_crouch {
                OrderType::WalkingUpright
            } else {
                OrderType::WalkingCrouched
            };
            s.push_back(walk(ctx.point_out, exit_action));
            s.push_back(passing_door());
        }
    } else {
        if ctx.direct {
            // Low, outside -> inside (climb UP).  Tolerance is the
            // `TransitionWaitingUprightClimbingWallUp` animation
            // distance, precomputed in `build_door_pass` and threaded
            // as `ctx.tol_wall_low_direct`.
            s.push_back(walk_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_wall_low_direct,
            ));
            s.push_back(walk_nodir(
                ctx.point_mid,
                OrderType::TransitionWaitingUprightClimbingWallUp,
            ));
            s.push_back(passing_door());
            s.push_back(walk_nodir(ctx.point_in, OrderType::ClimbingWallUp));
            s.push_back(passing_door());
        } else {
            // Low, inside -> outside (climb DOWN)
            s.push_back(walk_nodir(ctx.point_mid, OrderType::ClimbingWallDown));
            s.push_back(transition(
                OrderType::TransitionClimbingWallDownWaitingUpright,
            ));
            s.push_back(passing_door());
            if ctx.is_pc && ctx.sector_out_forces_crouch {
                s.push_back(transition(OrderType::TransitionCrouchingDown));
                s.push_back(walk(ctx.point_out, OrderType::WalkingCrouched));
            } else {
                s.push_back(walk(ctx.point_out, OrderType::WalkingUpright));
            }
            s.push_back(passing_door());
        }
    }
    s
}

// ─── Stairs door translation ────────────────────────────────────────

fn translate_stairs(ctx: &DoorPassContext) -> VecDeque<DoorPassStep> {
    let mut s = VecDeque::new();
    let reverse = ctx.is_carrying_on_shoulders;

    // Determine inside/outside animations based on current movement
    // action.  Sword / shield / corpse variants have no stairs-specific
    // animation — the same action plays for both the outside walk-to-mid
    // segment and the inside walk-past segment.
    let (anim_outside, anim_inside) = match ctx.action {
        OrderType::WalkingUpright => (OrderType::WalkingUpright, OrderType::WalkingStairs),
        OrderType::RunningUpright => (OrderType::RunningUpright, OrderType::RunningStairs),
        OrderType::WalkingWithSword
        | OrderType::WalkingWithShield
        | OrderType::WalkingWithCorpse => (ctx.action, ctx.action),
        other => (other, other),
    };

    if ctx.direct {
        // Outside -> inside
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_mid,
            action: anim_outside,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_in,
            action: anim_inside,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());
    } else {
        // Inside -> outside
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_mid,
            action: anim_inside,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_out,
            action: anim_outside,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());
    }
    s
}

// ─── Translate default/gate/trap/reinforcement doors ────────────────

fn translate_default(ctx: &DoorPassContext) -> VecDeque<DoorPassStep> {
    let mut s = VecDeque::new();
    let reverse = ctx.is_carrying_on_shoulders;
    let action = ctx.action;

    if !ctx.direct {
        // Inside -> outside
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_mid,
            action,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());

        // Forced-crouch on exit sector
        if ctx.is_pc && ctx.sector_out_forces_crouch {
            s.push_back(transition(OrderType::TransitionCrouchingDown));
            s.push_back(walk(ctx.point_out, OrderType::WalkingCrouched));
        } else {
            s.push_back(DoorPassStep::Walk {
                destination: ctx.point_out,
                action,
                reverse,
                compute_direction: true,
                tolerance: 0.0,
            });
        }
        s.push_back(passing_door());
    } else {
        // Outside -> inside
        s.push_back(DoorPassStep::Walk {
            destination: ctx.point_mid,
            action,
            reverse,
            compute_direction: true,
            tolerance: 0.0,
        });
        s.push_back(passing_door());

        // Forced-crouch on entry sector
        if ctx.is_pc && ctx.sector_in_forces_crouch {
            s.push_back(transition(OrderType::TransitionCrouchingDown));
            s.push_back(walk(ctx.point_in, OrderType::WalkingCrouched));
        } else {
            s.push_back(DoorPassStep::Walk {
                destination: ctx.point_in,
                action,
                reverse,
                compute_direction: true,
                tolerance: 0.0,
            });
        }
        s.push_back(passing_door());
    }
    s
}

/// Return value from [`PassDoorLaunchContext::build_door_pass`].
///
/// Pairs the built step chain with a post-install action-recursive
/// override.  When the PC exits a ladder/wall pass into a forced-crouch
/// sector, the element's root action must be rewritten to
/// `WalkingCrouched`.  The caller applies the override via
/// `SequenceManager::set_action_recursive` after the PassDoor element
/// is installed so the element's root action reads WalkingCrouched
/// instead of the upstream-chosen `ctx.action`.
struct BuiltDoorPass {
    pass: ActiveDoorPass,
    root_action: OrderType,
    post_chain_action_recursive: Option<OrderType>,
}

/// Whether PassDoor launch reaches the synchronous-successor splice in the
/// sequence action loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassDoorLaunchBarrier {
    ReachSplice,
    SkipSplice,
}

/// Canonical PassDoor launch state.
///
/// Original provenance: `RHelementactor.cpp:3493+` disables anti-collision,
/// resolves direction from the actor's side of the door, authorizes the actor,
/// translates the door/lift variant, then starts the first movement order
/// synchronously. No script VM or mission mirror participates in that
/// translation.
pub(super) struct PassDoorLaunchContext<'a> {
    doors: &'a [crate::gate::Door],
    entities: &'a mut crate::entities::Entities,
    fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl<'a> PassDoorLaunchContext<'a> {
    pub(super) fn new(
        doors: &'a [crate::gate::Door],
        entities: &'a mut crate::entities::Entities,
        fast_grid: &'a crate::fast_find_grid::FastFindGrid,
        sequence_manager: &'a mut crate::sequence::SequenceManager,
        next_order_id: &'a mut u32,
    ) -> Self {
        Self {
            doors,
            entities,
            fast_grid,
            sequence_manager,
            next_order_id,
        }
    }

    pub(super) fn dispatch(
        &mut self,
        entity_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> PassDoorLaunchBarrier {
        let movement = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement {
                    gate_id,
                    flags,
                    direction,
                    action,
                    ..
                } => Some((
                    *gate_id,
                    *flags,
                    *direction,
                    *action,
                    element.posture_after_transition,
                    element.action_state_after_transition,
                    element.legacy_v48.is_some(),
                )),
                _ => None,
            });
        let (
            gate_id,
            flags,
            saved_direction,
            authored_action,
            posture_after_transition,
            action_state_after_transition,
            restored_from_v48,
        ) = movement.unwrap_or_else(|| {
            panic!(
                "PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} is not movement data"
            )
        });
        let door_index = gate_id.unwrap_or_else(|| {
            panic!("PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} has no gate")
        });
        let door = self
            .doors
            .get(usize::from(door_index))
            .unwrap_or_else(|| {
                panic!(
                    "PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} references missing door {door_index}"
                )
            });

        let (actor_sector, auth_info) = self
            .entities
            .get(entity_id)
            .map(|entity| (entity.element_data().sector(), entity.actor_auth_info()))
            .unwrap_or_else(|| {
                panic!(
                    "PassDoor sequence element {seq_id:?}/{elem_idx} references missing owner {entity_id:?}"
                )
            });

        // `RHelementactor.cpp:3493+` disables anti-collision before the
        // direction/authorization switch. Denied and otherwise-impossible
        // attempts therefore leave it disabled until movement teardown.
        self.entities
            .get_mut(entity_id)
            .expect("PassDoor owner disappeared between canonical lookups")
            .position_iface_mut()
            .set_anti_collision_on(false);

        let actor_sector = actor_sector.unwrap_or_else(|| {
            panic!(
                "PassDoor owner {entity_id:?} has no sector for door {door_index} direction resolution at {seq_id:?}/{elem_idx}"
            )
        });
        let direct = if u16::from(actor_sector) == u16::from(door.sector_out) {
            true
        } else if u16::from(actor_sector) == u16::from(door.sector_in) {
            false
        } else {
            panic!(
                "PassDoor owner {entity_id:?} sector {actor_sector} is on neither side of door {door_index} (out {}, in {}) at {seq_id:?}/{elem_idx}",
                door.sector_out, door.sector_in
            )
        };
        let allow_leave_map = flags.contains(crate::sequence::MoveFlags::MAP);
        // Building capacity is always effectively unlimited in the loaded
        // game data; this preserves the previous dispatcher contract.
        if !door.is_actor_authorized(direct, &auth_info, true, allow_leave_map) {
            tracing::debug!(
                entity = ?entity_id,
                door = %door_index,
                ?direct,
                "PassDoor: actor not authorized"
            );
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return PassDoorLaunchBarrier::SkipSplice;
        }
        let lift_type = match door.door_type {
            DoorType::LiftHigh | DoorType::LiftLow | DoorType::LiftHighCrenel => self
                .grid_sector_by_number(door.sector_in)
                .and_then(|sector| sector.lift_type),
            _ => None,
        };
        if lift_type.is_some_and(|lift_type| !lift_type.is_actor_authorized(&auth_info)) {
            tracing::debug!(
                entity = ?entity_id,
                door = %door_index,
                ?direct,
                "PassDoor: actor not authorized for lift type"
            );
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return PassDoorLaunchBarrier::SkipSplice;
        }

        let mut built = self.build_door_pass(
            entity_id,
            door_index,
            direct,
            flags,
            authored_action,
            posture_after_transition,
            action_state_after_transition,
        );
        // A live PassDoor writes its traversal direction onto the movement
        // element before the actor crosses the gate. That field survives a
        // v48 save, while the actor sector may already be the destination
        // side. Rebuild the remaining physical steps from the live sector,
        // but retain the element's C++ truth value for `Position(actor)` and
        // committed route-source queries.
        if restored_from_v48 {
            built.pass.position_direct = saved_direction != 0;
        }
        // The original PassDoor translation only rewrites the movement
        // element's action for building / default / gate / trap doors and
        // for stairs lifts, and only on the element itself.  Ladder and
        // wall lift passes leave the element's authored action untouched
        // (their step actions are explicit), and nothing propagates to
        // following elements — only the PC forced-crouch override below
        // walks the chain recursively.
        let rewrite_element_action = match door.door_type {
            DoorType::LiftHigh | DoorType::LiftLow | DoorType::LiftHighCrenel => {
                matches!(lift_type, Some(LiftType::Stairs) | Some(LiftType::Normal))
            }
            _ => true,
        };
        if rewrite_element_action
            && let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx)
        {
            elem.set_action(built.root_action);
        }
        if let Some(override_action) = built.post_chain_action_recursive {
            self.sequence_manager
                .set_action_recursive(seq_id, elem_idx, override_action);
        }
        let Some(DoorPassStep::Walk {
            destination,
            action,
            reverse,
            compute_direction,
            tolerance,
        }) = built.pass.steps.pop_front()
        else {
            panic!(
                "PassDoor translation for owner {entity_id:?}, door {door_index} did not start with a Walk step"
            );
        };
        built.pass.current_action = action;
        built.pass.current_reverse = reverse;
        self.install_initial_walk(
            entity_id,
            seq_id,
            elem_idx,
            destination,
            action,
            reverse,
            compute_direction,
            tolerance,
            built.pass,
        );
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
        tracing::debug!(
            entity = ?entity_id,
            door = %door_index,
            ?direct,
            "PassDoor: started multi-step door pass"
        );
        PassDoorLaunchBarrier::ReachSplice
    }

    fn grid_sector_by_number(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> Option<&crate::fast_find_grid::GridSector> {
        self.fast_grid
            .level
            .sector_number_map
            .get(&sector_number)
            .and_then(|&index| self.fast_grid.level.sectors.get(index))
    }

    fn sector_forces_crouch(&self, sector_number: crate::sector::SectorNumber) -> bool {
        self.grid_sector_by_number(sector_number)
            .unwrap_or_else(|| {
                panic!("PassDoor references missing canonical sector {sector_number}")
            })
            .force_crouched
    }
}

// ─── Misc helpers ───────────────────────────────────────────────────

/// Start the hulk flash on a humanoid element with default outline,
/// width 2, and the given fade speed.
pub(super) fn start_hulk_on(entity: &mut crate::element::Entity, speed: f32) {
    let elem = entity.element_data_mut();
    elem.current_outline = crate::element::OutlineColorName::Default;
    elem.outline_width = 2;
    if let Some(human) = entity.human_data_mut() {
        human.start_hulk(true, speed);
    }
}

// ─── EngineInner methods ─────────────────────────────────────────────────

impl PassDoorLaunchContext<'_> {
    /// Build the complete door-pass step chain for the given door and actor.
    ///
    /// Dispatches to the appropriate translate function based on door type
    /// and lift type.
    fn build_door_pass(
        &self,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
        flags: crate::sequence::MoveFlags,
        authored_action: OrderType,
        posture_after_transition: Posture,
        action_state_after_transition: crate::element::ActionState,
    ) -> BuiltDoorPass {
        // Snapshot canonical door geometry and type.
        let (door_type, pt_mid, pt_in, pt_out, sector_in, door_sector_out) = {
            let door = self.doors.get(usize::from(door_index)).unwrap_or_else(|| {
                panic!("PassDoor build for {entity_id:?} references missing door {door_index}")
            });
            (
                door.door_type,
                door.point_mid,
                door.point_in,
                door.point_out,
                door.sector_in,
                door.sector_out,
            )
        };

        // Read actor properties.
        let entity = self
            .entities
            .get(entity_id)
            .unwrap_or_else(|| panic!("PassDoor build references missing owner {entity_id:?}"));
        let is_pc = entity.is_pc();
        let is_soldier = entity.is_soldier();
        let is_carrying = posture_after_transition == Posture::CarryingOnShoulders;

        // Soldier attentive state: the soldier AI's persistent attentive
        // flag (set/cleared by enter/leave-attentive transitions), not a
        // derived property of the current action state.  Attentive
        // soldiers use the alerted ladder climb transition animations.
        let is_attentive = is_soldier && entity.enemy_ai().is_some_and(|enemy| enemy.attentive);

        // Choose base movement animation. Original Human/PC
        // `DetermineMovementAnimation` switches on the posture/action state
        // stamped onto the movement element by Actor::Instruct, not the live
        // actor state. It starts from the authored action; FAST remains an
        // independent path flag and does not imply that an AI-authored
        // `RunningUpright` PassDoor should walk.
        // Door-pass uses the `WalkingWith*` / `RunningWith*` variants so
        // the stairs translator routes them through the sword/shield
        // branch instead of the plain walk/run branch.
        let is_fast = flags.contains(crate::sequence::MoveFlags::FAST);
        let mut action = if posture_after_transition == Posture::Crouched {
            // Original's base DetermineMovementAnimation switches on the
            // movement element's post-transition posture before adapting its
            // authored action. A crouched PassDoor therefore remains
            // WalkingCrouched, including across a stairs lift.
            OrderType::WalkingCrouched
        } else if posture_after_transition == Posture::CarryingCorpse {
            // Carrying a corpse is another unconditional base-actor posture
            // rewrite.  In particular, an authored fast run remains
            // WalkingWithCorpse through every translated door rail.
            OrderType::WalkingWithCorpse
        } else if is_carrying {
            OrderType::WalkingCarryingOnShoulders
        } else if action_state_after_transition.is_sword() {
            // Human::DetermineMovementAnimation derives sword speed from
            // the authored movement action, not RHMOVE_FAST. PassDoor
            // elements deliberately carry empty flags, so consulting FAST
            // here downgraded an authored run at each door boundary.
            match authored_action {
                OrderType::WalkingWithSword | OrderType::RunningWithSword => authored_action,
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    OrderType::WalkingWithSword
                }
                OrderType::RunningUpright => OrderType::RunningWithSword,
                other => panic!(
                    "DetermineMovementAnimation received unsupported sword PassDoor action {other:?} for {entity_id:?}"
                ),
            }
        } else if is_pc && action_state_after_transition.is_shield() {
            // No running-with-shield variant — shield posture is
            // always a walk regardless of the fast flag.
            OrderType::WalkingWithShield
        } else {
            match authored_action {
                OrderType::WalkingUpright
                | OrderType::RunningUpright
                | OrderType::RiderCharging => authored_action,
                OrderType::WalkingStairs => OrderType::WalkingUpright,
                OrderType::WalkingCrouched
                | OrderType::ClimbingWallUp
                | OrderType::ClimbingWallDown
                | OrderType::ClimbingLadderUp
                | OrderType::ClimbingLadderDown
                | OrderType::ClimbingLadderUpFast
                | OrderType::ClimbingLadderDownFast
                | OrderType::ClimbingWallUpFast
                | OrderType::ClimbingWallDownFast
                | OrderType::WalkingCarryingOnShoulders => {
                    if is_fast {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    }
                }
                OrderType::WalkingWithSword if is_pc => OrderType::WalkingUpright,
                OrderType::RunningWithSword if is_pc => OrderType::RunningUpright,
                other => other,
            }
        };
        let destination = if direct { pt_in } else { pt_out };
        // The sword branch above (and the PC-only shield branch) is the arm a
        // derived actor resolves entirely on its own — it never reaches the
        // base implementation that asks the lift sector to translate the
        // action. Running the translation anyway collapses the combat token
        // to the stairs walk, so an armed soldier crossing a stairs door
        // loses its sword animation for the whole pass.
        let derived_override_is_authoritative = matches!(
            action,
            OrderType::WalkingWithSword | OrderType::RunningWithSword
        ) || (is_pc
            && action == OrderType::WalkingWithShield);
        if !derived_override_is_authoritative {
            action = super::movement::determine_lift_movement_animation_for(
                entity,
                self.fast_grid,
                posture_after_transition,
                action,
                destination,
            );
        }

        let sector_out_forces_crouch = self.sector_forces_crouch(door_sector_out);
        let sector_in_forces_crouch = self.sector_forces_crouch(sector_in);

        // Determine lift type for lift doors.
        let lift_type = match door_type {
            DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::LiftLow => self
                .grid_sector_by_number(sector_in)
                .and_then(|sector| sector.lift_type),
            _ => None,
        };
        let is_high = matches!(door_type, DoorType::LiftHigh | DoorType::LiftHighCrenel);
        let is_crenel = door_type == DoorType::LiftHighCrenel;

        // All five tolerance values used by the ladder/wall translators
        // are precomputed here via `Sprite::distance_for_animation` so
        // the translator functions stay sprite-free.
        //
        // The high/direct ladder sums are wrapped in `abs(...)` because
        // the climb-down transition distance is negative; the wall and
        // low/direct ladder tolerances are used raw.
        let sprite = entity.sprite();
        let dist = |anim: OrderType| -> f32 { f32::from(sprite.distance_for_animation(anim)) };
        let tol_ladder_high_direct = if is_attentive {
            // Soldier + attentive
            dist(OrderType::TransitionWaitingUprightClimbingLadderDownAlerted).abs()
        } else if is_soldier {
            // Soldier + not attentive
            dist(OrderType::TransitionWaitingCrouchedClimbingLadderDown).abs()
        } else {
            // Non-soldier: crouching-down + climbing-down sum
            (dist(OrderType::TransitionCrouchingDown)
                + dist(OrderType::TransitionWaitingCrouchedClimbingLadderDown))
            .abs()
        };
        let tol_ladder_low_direct = dist(OrderType::TransitionWaitingUprightClimbingLadderUp);
        let tol_wall_high_direct_noncrenel = dist(OrderType::TransitionCrouchingDown)
            + dist(OrderType::TransitionWaitingCrouchedClimbingWallDown);
        let tol_wall_high_direct_crenel =
            dist(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel);
        let tol_wall_low_direct = dist(OrderType::TransitionWaitingUprightClimbingWallUp);

        let ctx = DoorPassContext {
            door_type,
            point_mid: pt_mid,
            point_in: pt_in,
            point_out: pt_out,
            direct,
            is_pc,
            is_soldier_attentive: is_attentive,
            action,
            is_carrying_on_shoulders: is_carrying,
            sector_out_forces_crouch,
            sector_in_forces_crouch,
            is_high,
            is_crenel,
            tol_ladder_high_direct,
            tol_ladder_low_direct,
            tol_wall_high_direct_noncrenel,
            tol_wall_high_direct_crenel,
            tol_wall_low_direct,
        };

        // Gate-type doors transition `gate_state` inside
        // `apply_door_patch`: the direction (open vs close) is read
        // off the patch's `applied` flag, so `gate_state` always
        // matches the current visual regardless of which side of the
        // open/close cycle we're on.  No pre-emptive call is needed
        // here — the patch's applied-ness *is* the gate's state.
        let _ = door_type;

        let steps = match door_type {
            DoorType::Building | DoorType::BuildingTrap => translate_building(&ctx),
            DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::LiftLow => match lift_type {
                Some(LiftType::Ladder) => translate_ladder(&ctx),
                Some(LiftType::Wall) => translate_wall(&ctx),
                Some(LiftType::Stairs) | Some(LiftType::Normal) => translate_stairs(&ctx),
                None => panic!(
                    "PassDoor owner {entity_id:?} door {door_index} is a lift door but sector {sector_in} has no lift type"
                ),
            },
            _ => translate_default(&ctx),
        };

        // When the PC exits a ladder/wall pass (non-direct) into a
        // forced-crouch sector, rewrite the PassDoor movement
        // element's root action to `WalkingCrouched` so any future
        // order appended to the element reads the post-crouch action.
        let post_chain_action_recursive = if is_pc
            && !direct
            && sector_out_forces_crouch
            && matches!(lift_type, Some(LiftType::Ladder) | Some(LiftType::Wall))
        {
            Some(OrderType::WalkingCrouched)
        } else {
            None
        };

        BuiltDoorPass {
            pass: ActiveDoorPass {
                door_index,
                direct,
                position_direct: direct,
                steps,
                triggers_fired: 0,
                current_action: action,
                current_reverse: false,
                saved_action_state: None,
            },
            root_action: action,
            post_chain_action_recursive,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn install_initial_walk(
        &mut self,
        entity_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        tolerance: f32,
        active_door_pass: ActiveDoorPass,
    ) {
        let door_handle = crate::position_interface::DoorHandle(active_door_pass.door_index.0);
        let door_direction = active_door_pass.direct;
        let order_id = crate::order::alloc_order_id(self.next_order_id);
        let mut order = crate::order::Order::new(action, destination.x, destination.y, order_id);
        order.reverse = reverse;
        order.compute_direction = compute_direction;
        order.tolerance = tolerance;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);

        let entity = self.entities.get_mut(entity_id).unwrap_or_else(|| {
            panic!(
                "PassDoor initial walk for {entity_id:?} at {seq_id:?}/{elem_idx} lost its actor"
            )
        });
        // Original Translate installs the door pointer before constructing
        // the translated order chain. IsInsideBuilding observes that pointer
        // throughout the pre-trigger rail movement, even while the actor's
        // sector still names the outdoor side.
        entity
            .position_iface_mut()
            .set_door(door_handle, door_direction);
        let actor = entity.actor_data_mut().unwrap_or_else(|| {
            panic!(
                "PassDoor initial walk for {entity_id:?} at {seq_id:?}/{elem_idx} lost its actor"
            )
        });
        actor.active_movement = crate::movement::ActiveMovement::new(seq_id, elem_idx);
        actor.passing_door_directly = active_door_pass.position_direct;
        actor.active_door_pass = Some(active_door_pass);
        actor.sequence_element_started = true;
    }
}

// ─── Engine completion methods ─────────────────────────────

impl EngineInner {
    /// Execute the PassDoor callback — change layer/sector and trigger
    /// building/lift callbacks.
    ///
    /// Called when a [`DoorPassStep::PassingDoor`] step fires.
    /// First call (trigger 0) changes layer/sector; subsequent calls
    /// re-enable anti-collision.
    pub(super) fn execute_pass_door(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
        trigger_number: u8,
    ) {
        if trigger_number > 0 {
            // Second (and later) trigger:
            // RHNONANIMATION_PASSING_DOOR calls SetAntiCollisionOn(true)
            // once PassDoor() has already consumed the gate.
            self.get_entity_mut(entity_id)
                .unwrap_or_else(|| {
                    panic!(
                        "PassDoor anti-collision callback for door {door_index} lost owner {entity_id:?}"
                    )
                })
                .position_iface_mut()
                .set_anti_collision_on(true);
            return;
        }

        // ── First trigger: perform the layer/sector change ──

        // Snapshot door data before mutable borrows.
        let (target_layer, target_sector_num, _door_type, is_lift_high, door_point_out) = {
            let door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
                    panic!(
                        "PassDoor callback for {entity_id:?} references missing door {door_index}"
                    )
                });
            let (tl, ts) = if direct {
                (door.layer_in, door.sector_in)
            } else {
                (door.layer_out, door.sector_out)
            };
            let is_high = matches!(
                door.door_type,
                DoorType::LiftHigh | DoorType::LiftHighCrenel
            );
            let pout = door.point_out;
            (tl, ts, door.door_type, is_high, pout)
        };

        // Read entity's current sector and type before the change.
        let (current_sector, is_pc) = self
            .get_entity(entity_id)
            .map(|entity| {
                (
                    entity.element_data().sector().unwrap_or_else(|| {
                        panic!(
                            "PassDoor callback for {entity_id:?}, door {door_index} has no source sector"
                        )
                    }),
                    entity.is_pc(),
                )
            })
            .unwrap_or_else(|| {
                panic!("PassDoor callback for door {door_index} lost owner {entity_id:?}")
            });
        let actor_handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
        let expected_source = if direct {
            self.script_domains.interactables.doors[usize::from(door_index)].sector_out
        } else {
            self.script_domains.interactables.doors[usize::from(door_index)].sector_in
        };
        assert_eq!(
            u16::from(current_sector),
            u16::from(expected_source),
            "PassDoor callback for {entity_id:?}, door {door_index} ran from sector {current_sector}, expected {expected_source}"
        );

        // ── Leave callbacks ──
        // Track whether we're leaving a building so we can refresh the
        // actor's projection-area obstacle + footstep material after the
        // layer/sector change: on building exit we re-seat the actor
        // onto the projection area at the door's outside point so the
        // next footstep sounds use the correct material.
        let mut left_building = false;
        {
            let cur_sector_num: u16 = current_sector.into();
            let gs = self
                .grid_sector_by_number(crate::sector::SectorNumber::new(cur_sector_num as i16))
                .unwrap_or_else(|| {
                    panic!(
                        "PassDoor callback for {entity_id:?}, door {door_index} references missing source sector {cur_sector_num}"
                    )
                });

            if gs.sector_type.is_building() {
                left_building = true;
                // Leaving a building — remove from occupant list.
                let bld_idx = Some(gs.building_index.unwrap_or_else(|| {
                    panic!(
                        "PassDoor owner {entity_id:?} left building sector {cur_sector_num} without a building index"
                    )
                }));
                if let Some(bi) = bld_idx {
                    let occupants = self
                        .script_domains
                        .buildings
                        .occupants
                        .get_mut(usize::from(bi))
                        .unwrap_or_else(|| {
                            panic!(
                                "PassDoor owner {entity_id:?} left building {bi} without an occupant list"
                            )
                        });
                    let old_len = occupants.len();
                    occupants.retain(|&a| a != actor_handle);
                    assert_ne!(
                        occupants.len(),
                        old_len,
                        "PassDoor owner {entity_id:?} was absent from building {bi} occupants on leave"
                    );
                    self.script_domains
                        .buildings
                        .actor_building
                        .remove(&actor_handle);
                }
                // Drop this entity from the matching `AiGlobalState`
                // house's live occupant list.  Keyed by sector number
                // (the `House::sector_index` field), not building
                // index.
                for house in self.ai.global.houses.iter_mut() {
                    if house.sector_index == cur_sector_num as u32 {
                        house.occupant_ids.retain(|&e| e != entity_id);
                        break;
                    }
                }
                // Re-show the actor sprite now that they've left the building.
                let carried_to_unhide = if let Some(entity) = self.get_entity_mut(entity_id) {
                    let elem = entity.element_data_mut();
                    elem.hidden_in_building = false;
                    elem.active = true;
                    // Carried corpse follows the carrier in/out of
                    // buildings — when the carrier becomes visible
                    // again, the carried entity must too.
                    entity.pc_data().and_then(|pc| pc.carried)
                } else {
                    None
                };
                if let Some(carried_id) = carried_to_unhide
                    && let Some(carried) = self.get_entity_mut(carried_id)
                {
                    let elem = carried.element_data_mut();
                    elem.hidden_in_building = false;
                    elem.active = true;
                }
                // When the leaving actor is a PC, (a) recursively
                // remove its carried actor (mirrors the Enter-side
                // push), and (b) if no PC remains in the building,
                // hide every other currently-visible occupant — the
                // corpses that the Enter side had unhidden go back
                // into "stored" state.
                if is_pc && let Some(bi) = bld_idx {
                    if let Some(carried_id) = carried_to_unhide {
                        let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
                        {
                            if let Some(occupants) = self
                                .script_domains
                                .buildings
                                .occupants
                                .get_mut(usize::from(bi))
                            {
                                occupants.retain(|&a| a != carried_h);
                            }
                            self.script_domains
                                .buildings
                                .actor_building
                                .remove(&carried_h);
                        }
                    }
                    // Snapshot the post-removal occupant list so we can
                    // probe each occupant without holding the script borrow.
                    let occupants: Vec<i32> = self
                        .script_domains
                        .buildings
                        .occupants
                        .get(usize::from(bi))
                        .cloned()
                        .unwrap_or_default();
                    let any_pc_remains = occupants.iter().any(|&h| {
                        self.entity_id_for_actor_handle(h)
                            .and_then(|id| self.world.entities.get(id))
                            .is_some_and(|e| e.is_pc())
                    });
                    if !any_pc_remains {
                        for occ_h in occupants {
                            let Some(occ_id) = self.entity_id_for_actor_handle(occ_h) else {
                                continue;
                            };
                            let Some(occ) = self.world.entities.get_mut(occ_id) else {
                                continue;
                            };
                            let elem = occ.element_data_mut();
                            if !elem.hidden_in_building {
                                elem.hidden_in_building = true;
                            }
                            elem.active = false;
                        }
                    }
                }
                // PassHouseDoor(false) hook on enemy AI.  The body is
                // currently empty, but the call is wired so future
                // PassHouseDoor logic flows naturally.
                if let Some(entity) = self.get_entity_mut(entity_id)
                    && let Some(ai) = entity.enemy_ai_mut()
                {
                    ai.pass_house_door(false);
                }
                tracing::debug!(entity = ?entity_id, sector = cur_sector_num, "PassDoor: left building");
            } else if gs.sector_type.is_lift() {
                // Leaving a lift — clear occupancy direction.
                if let Some(grid_idx) = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(cur_sector_num as i16))
                    .copied()
                    && self
                        .world
                        .fast_grid
                        .level
                        .sectors
                        .get(grid_idx)
                        .and_then(|gs| gs.lift_type)
                        .map(|lt| lt.is_wall_or_ladder())
                        .unwrap_or(false)
                {
                    let is_pc = self
                        .get_entity(entity_id)
                        .unwrap_or_else(|| {
                            panic!("PassDoor lift occupant {entity_id:?} vanished before release")
                        })
                        .is_pc();
                    let st = self.world.fast_grid_mut().lift_state_mut(grid_idx as u32);
                    if is_lift_high {
                        st.set_occupied_upwards(false, is_pc);
                    } else {
                        st.set_occupied_downwards(false, is_pc);
                    }
                }
                // Clear the actor's active_lift marker — they're no
                // longer mid-climb, so a subsequent push doesn't try
                // to decrement this sector a second time.
                if let Some(entity) = self.get_entity_mut(entity_id)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.active_lift = None;
                }
                tracing::debug!(entity = ?entity_id, sector = cur_sector_num, "PassDoor: left lift");
            }
        }

        // ── Change layer/sector on entity ──
        self.grid_sector_by_number(target_sector_num)
            .unwrap_or_else(|| {
                panic!(
                    "PassDoor callback for {entity_id:?}, door {door_index} references missing target sector {target_sector_num}"
                )
            });
        let entity = self
            .get_entity_mut(entity_id)
            .expect("PassDoor owner disappeared after canonical callback lookup");
        let elem = entity.element_data_mut();
        elem.set_layer(target_layer);
        elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
            target_sector_num,
        )));
        // RHElementActor::PassDoor consumes the sprite door pointer on the
        // first PassingDoor callback. A later callback only restores
        // anti-collision and must continue to observe a null door.
        elem.sprite.position_iface.clear_door();
        tracing::debug!(
            entity_id = ?entity_id,
            layer = target_layer,
            sector = %target_sector_num,
            "PassDoor: changed layer/sector"
        );

        // Refresh paired jump lines unconditionally on every sector
        // swap so swordfighters across a jump line re-evaluate their
        // per-opponent paired jump lines for the new sector.
        self.update_opponents_jump_lines(assets, entity_id);

        // ── Building-exit material / obstacle refresh ──
        // After leaving a building and switching to the outside sector,
        // re-seat the actor onto the appropriate projection-area
        // obstacle at the door's outside point so the next 1-2 footstep
        // sounds use the correct material (grass / stone / wood / ...).
        // Building exit is always `!_direct`, so the target sector is
        // `sector_out`.
        if left_building {
            let new_obstacle = self.find_projection_area_at(
                assets,
                target_layer,
                u16::from(target_sector_num),
                door_point_out,
            );
            self.set_obstacle_and_material(assets, entity_id, new_obstacle);
        }

        // ── Enter callbacks ──
        let enter_gs = self
            .grid_sector_by_number(target_sector_num)
            .expect("PassDoor target sector disappeared after canonical lookup");
        if enter_gs.sector_type.is_building() {
            // Entering a building — add to occupant list.
            let bld_idx = enter_gs.building_index.unwrap_or_else(|| {
                panic!(
                    "PassDoor owner {entity_id:?} entered building sector {target_sector_num} without a building index"
                )
            });
            let bld_handle =
                crate::natives::ScriptHandleCodec::building_handle_from_index(usize::from(bld_idx));
            self.script_domains
                .buildings
                .occupants
                .get_mut(usize::from(bld_idx))
                .unwrap_or_else(|| {
                    panic!(
                        "PassDoor owner {entity_id:?} entered building {bld_idx} without an occupant list"
                    )
                })
                .push(actor_handle);
            self.script_domains
                .buildings
                .actor_building
                .insert(actor_handle, bld_handle);
            // Add this entity to the matching `AiGlobalState` house's
            // live occupant list.  If no house exists for this
            // sector — either because the building has no plain
            // `Building` doors (e.g. mission-scripted portal
            // entries), or the init scan missed it — we skip the
            // update rather than synthesising a door-less house.
            for house in self.ai.global.houses.iter_mut() {
                if house.sector_index == u32::from(u16::from(target_sector_num)) {
                    if !house.occupant_ids.contains(&entity_id) {
                        house.occupant_ids.push(entity_id);
                    }
                    break;
                }
            }
            // Hide the actor sprite inside the building.
            let carried_to_hide = if let Some(entity) = self.get_entity_mut(entity_id) {
                let elem = entity.element_data_mut();
                elem.hidden_in_building = true;
                elem.active = false;
                // Special case: a PC carrying a corpse drags the body
                // into the building too — also hidden.
                entity.pc_data().and_then(|pc| pc.carried)
            } else {
                None
            };
            if let Some(carried_id) = carried_to_hide
                && let Some(carried) = self.get_entity_mut(carried_id)
            {
                let elem = carried.element_data_mut();
                elem.hidden_in_building = true;
                elem.active = false;
            }
            // When the entering actor is a PC, (a) recursively enter
            // its carried actor — which adds it to the occupant list —
            // and (b) re-enable existing occupants who are dead /
            // unconscious and not being carried so their corpses render
            // to the freshly-arrived PC.  Matches the script-side
            // `PutActorInBuilding` helper.
            if is_pc {
                if let Some(carried_id) = carried_to_hide {
                    let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
                    let bld_handle = crate::natives::ScriptHandleCodec::building_handle_from_index(
                        usize::from(bld_idx),
                    );
                    self.script_domains.buildings.occupants[usize::from(bld_idx)].push(carried_h);
                    self.script_domains
                        .buildings
                        .actor_building
                        .insert(carried_h, bld_handle);
                }
                // Re-enable corpses already inside the building: walk
                // the occupant list and unhide humans that are
                // (dead || unconscious) && not currently carried.
                let occupants: Vec<i32> = self
                    .script_domains
                    .buildings
                    .occupants
                    .get(usize::from(bld_idx))
                    .cloned()
                    .expect("building occupant list was required above");
                for occ_h in occupants {
                    let Some(occ_id) = self.entity_id_for_actor_handle(occ_h) else {
                        continue;
                    };
                    let Some(occ) = self.world.entities.get_mut(occ_id) else {
                        continue;
                    };
                    let Some(hd) = occ.human_data() else { continue };
                    let is_dead_or_ko = occ.is_dead() || hd.unconscious;
                    let has_carrier = hd.carrier.is_some();
                    if is_dead_or_ko && !has_carrier {
                        let elem = occ.element_data_mut();
                        elem.hidden_in_building = false;
                        elem.active = true;
                    }
                }
            }
            // PassHouseDoor(true) hook on enemy AI.  See the leave-side
            // comment for why the call is wired even though the body
            // is empty today.
            if let Some(entity) = self.get_entity_mut(entity_id)
                && let Some(ai) = entity.enemy_ai_mut()
            {
                ai.pass_house_door(true);
            }
            tracing::debug!(entity = ?entity_id, sector = %target_sector_num, "PassDoor: entered building (hidden)");
        }

        // ── Door patch application ──
        // Toggles the door's background tile patches (e.g. open/close
        // visual).
        self.apply_door_patch(sim, assets, door_index);

        // Applying the patch starts a transition animation on the
        // patch's FX entity.  `gate_state` is advanced from `Opening`
        // to `Open` — or `Closing` to `Closed` — when that transition
        // finishes, in the patch-transition-complete handler inside
        // the per-frame animation tick.  There is no explicit state
        // machine: the state *is* the patch's applied-ness, and the
        // visual *is* the transition animation.  The Rust enum is
        // driven off the same completion signal.

        let _ = is_pc;
    }

    /// Commit the exact endpoint when the final door-pass order completes.
    ///
    /// Original's last door-rail movement has already written this position
    /// before the derived NPC Hourglass tail runs. Keeping Rust's interpolated
    /// pre-door coordinate until a later movement pass makes same-slot AI
    /// callbacks observe the wrong side of the gate.
    pub(super) fn commit_completed_door_pass_position(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
    ) {
        let door = self
            .script_domains
            .interactables
            .doors
            .get(usize::from(door_index))
            .unwrap_or_else(|| {
                panic!("completed PassDoor for {entity_id:?} references missing door {door_index}")
            });
        let point = if direct {
            door.point_in
        } else {
            door.point_out
        };
        let point = crate::coordinates::MapPoint::new(point.x, point.y);
        // The final rail order normally reaches and snaps to the exact door
        // endpoint before PassDoor runs.  C++'s direct PassDoor branch then
        // changes only topology; it does not recompute any position
        // representation.  Reconstructing world Y as `map_y + z` and then
        // deriving map Y again as `world_y - z` is not bitwise idempotent at
        // every magnitude and can manufacture a one-ULP movement on the next
        // frame.  Keep the already-authoritative endpoint untouched.  The
        // fallback below remains necessary for paths whose final rail order
        // did not itself commit the endpoint.
        if self
            .get_entity(entity_id)
            .is_some_and(|entity| entity.element_data().position_map() == point)
        {
            return;
        }
        let position = if direct {
            // RHElementActor::PassDoor's direct branch changes topology but
            // does not call ComputePositionAll. The preceding door-rail
            // movement therefore owns Z; snapping map XY must not project it
            // through the currently installed plane a second time.
            let elevation = self
                .get_entity(entity_id)
                .unwrap_or_else(|| {
                    panic!(
                        "completed direct PassDoor for {entity_id:?} lost its owner before endpoint commit"
                    )
                })
                .element_data()
                .position()
                .z;
            let ground = crate::coordinates::GroundPoint::from_map_and_z(point, elevation);
            super::special_motion::SpecialMovePosition::World(
                crate::coordinates::WorldPoint3D::new(ground.x, ground.y, elevation),
            )
        } else {
            // The non-direct C++ branch does call ComputePositionAll, after
            // installing the outside projection plane on building exits.
            super::special_motion::SpecialMovePosition::Map(point)
        };
        self.finalize_special_move_position(
            assets,
            entity_id,
            position,
            None,
            None,
            (!direct).then_some(point),
            "completed door pass",
        );
    }

    pub(super) fn apply_completed_door_pass_lift_entry_state(
        &mut self,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
    ) {
        let door = self
            .script_domains
            .interactables
            .doors
            .get(usize::from(door_index))
            .unwrap_or_else(|| {
                panic!("completed PassDoor for {entity_id:?} references missing door {door_index}")
            });
        if !direct
            || !matches!(
                door.door_type,
                DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::LiftLow
            )
        {
            return;
        }
        let target_sector = door.sector_in;
        let sector = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(target_sector)))
            .unwrap_or_else(|| {
                panic!(
                    "completed PassDoor for {entity_id:?}, door {door_index} references missing lift sector {target_sector}"
                )
            });
        let lift_type = sector.lift_type.unwrap_or_else(|| {
            panic!(
                "completed PassDoor for {entity_id:?}, door {door_index} targets non-lift sector {target_sector}"
            )
        });
        let posture = match lift_type {
            LiftType::Wall => crate::element::Posture::OnWall,
            LiftType::Ladder => crate::element::Posture::OnLadder,
            _ => return,
        };

        let entity = self.get_entity_mut(entity_id).unwrap_or_else(|| {
            panic!("completed PassDoor for door {door_index} lost owner {entity_id:?}")
        });
        entity.set_posture(posture);
        entity
            .actor_data_mut()
            .unwrap_or_else(|| panic!("completed PassDoor owner {entity_id:?} is not an actor"))
            .action_state = crate::element::ActionState::Moving;
        tracing::debug!(
            entity = ?entity_id,
            sector = %target_sector,
            ?posture,
            "DoorPass: completed into lift, preserving climb state and facing"
        );
    }

    /// Find the projection-area obstacle in `sector_number` on `layer`
    /// that contains the given map-space point.
    ///
    /// Iterates the sector's projection-area obstacle list and returns
    /// the obstacle whose screen-space plane contains `point`.  When
    /// multiple candidates match, picks the one with the greatest
    /// top-plane height ("highest obstacle" disambiguation).  Returns
    /// `None` if no obstacle covers the point.
    ///
    /// The per-sector projection-area index isn't populated in the
    /// Rust port yet, so we fall back to scanning every sight obstacle
    /// flagged with a matching `(sector, layer)` pair — the static
    /// data stamped at load time in `engine::level_loading` (raw
    /// projection_area → `obs.sector` / `obs.layer`).
    pub(super) fn find_projection_area_at(
        &self,
        assets: &LevelAssets,
        layer: u16,
        sector_number: u16,
        point: crate::coordinates::MapPoint,
    ) -> Option<u16> {
        self.get_projection_area_index(assets, sector_number, layer, point)
    }

    /// Apply a projection-area obstacle + its footstep material to an
    /// actor.
    ///
    /// With `Some(obstacle_idx)`: the actor's sprite takes the
    /// obstacle's material and its top-plane coefficients.  With
    /// `None`: clears the obstacle and falls back to the sound-sector
    /// material at the actor's current position — iterate sound
    /// sectors and pick the material of the first one that contains
    /// the point, or the map's default material when none match.  The
    /// Rust port uses
    /// [`crate::material_sectors::MaterialSectors::material_at`] which
    /// encapsulates both steps.
    ///
    /// Updates both `ElementData` (obstacle_index, material) and the
    /// actor's `PositionInterface` (obstacle, plane, material).
    pub(super) fn set_obstacle_and_material(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        obstacle_index: Option<u16>,
    ) {
        let (material, plane) = match obstacle_index {
            Some(idx) => {
                let obs = self.sight_obstacles(assets).get(idx as usize);
                let material =
                    obs.map(|o| crate::element::GameMaterial::from_u32(o.material as u32));
                let plane = obs.map(|o| {
                    crate::position_interface::PlaneZCoeffs::from_plane_points(&o.top_plane_points)
                });
                (material, plane)
            }
            None => {
                // No obstacle: clear plane, then resolve footstep
                // material from the sound-sector list at the actor's
                // current map position, with the default material as
                // the fallback.
                let point = self
                    .get_entity(entity_id)
                    .map(|e| e.position_iface())
                    .map(|pi| pi.map_position());
                let material = point.map(|p| assets.material_sectors.material_at(p));
                (material, None)
            }
        };
        if let Some(entity) = self.get_entity_mut(entity_id) {
            if let Some(mat) = material {
                entity.element_data_mut().set_material(mat);
            }
            let pi = entity.position_iface_mut();
            pi.set_obstacle(
                obstacle_index.and_then(crate::position_interface::ObstacleHandle::new),
                plane,
            );
            if let Some(mat) = material {
                pi.set_material(mat);
            }
        }
    }

    /// Apply the patch associated with a door, if any.
    ///
    /// Calls `Patch::apply()` and delegates effect processing to
    /// `process_patch_effects` (patch_effects.rs).
    fn apply_door_patch(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        door_index: crate::gate::DoorIndex,
    ) {
        // Snapshot the patch_index from the door (avoid overlapping borrows).
        let patch_index = {
            match self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
            {
                Some(door) => door.patch_index,
                None => return,
            }
        };

        let patch_index = match patch_index {
            Some(idx) => idx,
            None => return, // Door has no associated patch
        };
        let patch_idx = usize::from(patch_index);

        // Drive the door's `gate_state` to match the direction the
        // patch is about to transition in: if the patch was
        // previously un-applied (drawbridge up, closed) it's now
        // opening; if it was applied (bridge down, open) it's now
        // closing.  The matching `finish_transition` fires when the
        // patch's FX animation ends in its live static-FX owner slot.
        let was_applied = {
            match self.script_domains.interactables.patches.get(patch_idx) {
                Some(p) => p.applied,
                None => return,
            }
        };
        if let Some(door) = self
            .script_domains
            .interactables
            .doors
            .get_mut(usize::from(door_index))
        {
            if was_applied {
                door.gate_state.request_close();
            } else {
                door.gate_state.request_open();
            }
        }

        // Apply the patch and collect effects.
        let effects = {
            let patch = match self.script_domains.interactables.patches.get_mut(patch_idx) {
                Some(p) => p,
                None => return,
            };
            patch.apply()
        };

        tracing::debug!(
            door = %door_index,
            patch = patch_idx,
            num_effects = effects.len(),
            "apply_door_patch: patch applied"
        );

        self.process_patch_effects(sim, assets, patch_index, effects);
    }

    /// Reset `already_selected` and start a hulk flash on the carrier
    /// and its carried body when a carry transition starts inside a
    /// building.
    pub(super) fn apply_carry_building_hulk(&mut self, carrier_id: EntityId, carried_id: EntityId) {
        let carrier_sector = self
            .get_entity(carrier_id)
            .and_then(|e| e.element_data().sector());
        let in_building = carrier_sector
            .and_then(|s| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(s)))
            })
            .map(|gs| gs.sector_type.is_building())
            .unwrap_or(false);
        if !in_building {
            return;
        }
        if let Some(carrier) = self.get_entity_mut(carrier_id) {
            if let Some(pc) = carrier.pc_data_mut() {
                pc.already_selected = false;
            }
            if let Some(human) = carrier.human_data_mut() {
                human.hulk_direction = true;
            }
        }
        if let Some(target) = self.get_entity_mut(carried_id) {
            start_hulk_on(target, 1.0);
        }
    }

    /// Fire the select hulk flash on a PC (and its carried target,
    /// if any).
    pub(super) fn apply_select_hulk(&mut self, entity_id: EntityId, speed: f32) {
        let carried = {
            let Some(entity) = self.get_entity_mut(entity_id) else {
                return;
            };
            start_hulk_on(entity, speed);
            entity.pc_data().and_then(|pc| pc.carried)
        };
        if let Some(cid) = carried
            && let Some(carried_entity) = self.get_entity_mut(cid)
        {
            start_hulk_on(carried_entity, speed);
        }
    }

    /// Look up a GridSector by its sector_number. Returns `None` if not found.
    pub(super) fn grid_sector_by_number(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> Option<&crate::fast_find_grid::GridSector> {
        self.world
            .fast_grid
            .level
            .sector_number_map
            .get(&sector_number)
            .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
    }

    /// Check whether a sector forces crouched movement.
    pub(super) fn sector_forces_crouch(&self, sector_num: crate::sector::SectorNumber) -> bool {
        self.grid_sector_by_number(sector_num)
            .map(|gs| gs.force_crouched)
            .unwrap_or(false)
    }

    /// Get the lift type for a sector, if it's a lift sector.
    pub(super) fn get_sector_lift_type(
        &self,
        sector_num: crate::sector::SectorNumber,
    ) -> Option<LiftType> {
        self.grid_sector_by_number(sector_num)
            .and_then(|gs| gs.lift_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData, NpcData,
        PcData, SoldierData,
    };
    use crate::engine::movement::DoorPassAdvance;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};

    fn make_soldier(sector: Option<u16>) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        if let Some(sector) = sector {
            element.set_sector(crate::position_interface::SectorHandle::new(sector));
        }
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn make_pc(sector: u16) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_sector(crate::position_interface::SectorHandle::new(sector));
        Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn dispatch_pass(
        engine: &mut EngineInner,
        doors: &[crate::gate::Door],
        owner: EntityId,
    ) -> (PassDoorLaunchBarrier, crate::sequence::SequenceId) {
        let (posture_after_transition, action_state_after_transition) = engine
            .world
            .entities
            .get(owner)
            .map(|entity| {
                (
                    entity.element_data().posture,
                    entity.actor_data().unwrap().action_state,
                )
            })
            .unwrap();
        dispatch_pass_with_transition_state(
            engine,
            doors,
            owner,
            OrderType::WalkingUpright,
            crate::sequence::MoveFlags::empty(),
            posture_after_transition,
            action_state_after_transition,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_pass_with_transition_state(
        engine: &mut EngineInner,
        doors: &[crate::gate::Door],
        owner: EntityId,
        authored_action: OrderType,
        flags: crate::sequence::MoveFlags,
        posture_after_transition: Posture,
        action_state_after_transition: crate::element::ActionState,
    ) -> (PassDoorLaunchBarrier, crate::sequence::SequenceId) {
        for sector_number in doors
            .iter()
            .flat_map(|door| [door.sector_out, door.sector_in])
        {
            if engine
                .world
                .fast_grid
                .level
                .sector_number_map
                .contains_key(&sector_number)
            {
                continue;
            }
            let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
            let index = level.sectors.len();
            level.sector_number_map.insert(sector_number, index);
            level.sectors.push(crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
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
        }
        let mut element = SequenceElement::new_movement(
            1,
            crate::element::Command::PassDoor,
            Some(owner),
            authored_action,
        );
        element.posture_after_transition = posture_after_transition;
        element.action_state_after_transition = action_state_after_transition;
        let SequenceElementData::Movement {
            gate_id,
            flags: element_flags,
            ..
        } = &mut element.data
        else {
            unreachable!()
        };
        *gate_id = Some(crate::gate::DoorIndex(0));
        *element_flags = flags;
        let seq_id = engine.orders.sequence_manager.launch_element(element);
        let barrier = PassDoorLaunchContext::new(
            doors,
            &mut engine.world.entities,
            &engine.world.fast_grid,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, seq_id, 0);
        (barrier, seq_id)
    }

    fn default_door() -> crate::gate::Door {
        crate::gate::Door {
            sector_out: crate::sector::SectorNumber::new(7),
            sector_in: crate::sector::SectorNumber::new(8),
            point_mid: MapPoint::new(20.0, 30.0),
            point_out: MapPoint::new(10.0, 30.0),
            point_in: MapPoint::new(30.0, 30.0),
            ..crate::gate::Door::default()
        }
    }

    fn install_lift_sector(engine: &mut EngineInner, lift_type: LiftType) {
        let lift_sector = crate::sector::SectorNumber::new(42);
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        let index = level.sectors.len();
        level.sector_number_map.insert(lift_sector, index);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number: lift_sector,
            door_index: None,
            lift_type: Some(lift_type),
            lift_direction: 5,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
    }

    #[test]
    fn fallback_wait_on_ladder_preserves_inherited_facing() {
        let mut engine = EngineInner::new();
        install_lift_sector(&mut engine, LiftType::Ladder);
        let owner = engine.add_entity(make_pc(42));
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.set_posture(Posture::OnLadder);
            entity.element_data_mut().set_direction_instantly(1);
        }

        engine.ensure_wait_element(owner);

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().direction(), 1);
        assert_eq!(entity.position_iface().get_direction_goal().as_u8(), 1);
        let element = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(owner) && element.command == crate::element::Command::Wait
            })
            .expect("fallback Wait must be installed");
        assert_eq!(element.command, crate::element::Command::Wait);
    }

    fn bind_single_animation(engine: &mut EngineInner, owner: EntityId, action: OrderType) {
        let mut conversion = vec![
            crate::sprite_script::UNMAPPED;
            crate::sprite_script::NONANIMATION_END.max(action as usize + 1)
        ];
        conversion[action as usize] = 0;
        let played_action = match action {
            OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
            OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
            OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
            OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
            other => other,
        };
        if played_action as usize >= conversion.len() {
            conversion.resize(played_action as usize + 1, crate::sprite_script::UNMAPPED);
        }
        conversion[played_action as usize] = 0;
        let script = crate::sprite_script::SpriteScript {
            action_id: action as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2, 3],
            delays: vec![10, 10, 10],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
        };
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(std::iter::repeat_n(script, 16).collect()),
            std::sync::Arc::new(conversion),
        );
    }

    fn install_production_climb_fixture(
        engine: &mut EngineInner,
        owner: EntityId,
        lift_type: LiftType,
        action: OrderType,
    ) -> (crate::sequence::SequenceId, std::num::NonZeroU32) {
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .has_climb = true;
        install_lift_sector(engine, lift_type);
        let door = crate::gate::Door {
            door_type: DoorType::LiftLow,
            sector_in: crate::sector::SectorNumber::new(42),
            ..default_door()
        };
        engine.script_domains.interactables.doors.push(door.clone());
        let (_, seq_id) = dispatch_pass(engine, &[door], owner);
        bind_single_animation(engine, owner, action);

        let order_id = {
            let order = engine
                .orders
                .sequence_manager
                .get_element_mut(seq_id, 0)
                .unwrap()
                .orders
                .front_mut()
                .unwrap();
            order.order_type = action;
            order.compute_direction = false;
            order.target_x = 20.0;
            order.target_y = 30.0;
            order.order_id
        };
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.element_data_mut().set_direction_instantly(0);
            entity
                .element_data_mut()
                .set_sector(crate::position_interface::SectorHandle::new(7));
            let actor = entity.actor_data_mut().unwrap();
            actor.action_state = crate::element::ActionState::Waiting;
            actor.active_door_pass.as_mut().unwrap().current_action = action;
        }
        engine.execute_pass_door(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            crate::gate::DoorIndex(0),
            true,
            0,
        );
        (seq_id, order_id)
    }

    #[test]
    fn launch_context_resolves_direction_and_installs_first_order_before_splice() {
        for (actor_sector, expected_direct, expected_exit) in [
            (7, true, MapPoint::new(30.0, 30.0)),
            (8, false, MapPoint::new(10.0, 30.0)),
        ] {
            let mut engine = EngineInner::new();
            let owner = engine.add_entity(make_soldier(Some(actor_sector)));
            let (barrier, seq_id) = dispatch_pass(&mut engine, &[default_door()], owner);

            assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
            let element = engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap();
            assert_eq!(element.state, SequenceState::InProgress);
            let order = element.current_order().expect("initial walk is installed");
            assert_eq!(order.order_type, OrderType::WalkingUpright);
            assert_eq!((order.target_x, order.target_y), (20.0, 30.0));
            let entity = engine.world.entities.get(owner).unwrap();
            assert!(!entity.position_iface().is_anti_collision_on());
            assert_eq!(
                entity.position_iface().get_door(),
                crate::position_interface::DoorHandle(0),
                "translated door movement must expose Original's sprite door pointer"
            );
            assert_eq!(
                entity.position_iface().get_door_direction(),
                expected_direct
            );
            let pass = entity
                .actor_data()
                .unwrap()
                .active_door_pass
                .as_ref()
                .expect("door pass is active before the splice");
            assert_eq!(pass.direct, expected_direct);
            let translated_exit = pass.steps.iter().find_map(|step| match step {
                DoorPassStep::Walk { destination, .. } => Some(*destination),
                _ => None,
            });
            assert_eq!(translated_exit, Some(expected_exit));
        }
    }

    #[test]
    fn corpse_carrying_pass_rewrites_authored_fast_run_without_affecting_upright_control() {
        for (posture, expected_action) in [
            (Posture::CarryingCorpse, OrderType::WalkingWithCorpse),
            (Posture::Upright, OrderType::RunningUpright),
        ] {
            let mut engine = EngineInner::new();
            let owner = engine.add_entity(make_pc(7));
            engine
                .world
                .entities
                .get_mut(owner)
                .expect("door-pass test PC")
                .set_posture(posture);

            let (barrier, seq_id) = dispatch_pass_with_transition_state(
                &mut engine,
                &[default_door()],
                owner,
                OrderType::RunningUpright,
                crate::sequence::MoveFlags::FAST,
                posture,
                crate::element::ActionState::MovingFast,
            );

            assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
            let element = engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("PassDoor element remains installed");
            let SequenceElementData::Movement { action, flags, .. } = &element.data else {
                unreachable!()
            };
            assert!(flags.contains(crate::sequence::MoveFlags::FAST));
            assert_eq!(*action, expected_action, "translated root action");
            assert_eq!(
                element.current_order().map(|order| order.order_type),
                Some(expected_action),
                "the selected first door rail must use the posture-adapted action"
            );

            let pass = engine
                .world
                .entities
                .get(owner)
                .expect("door-pass test PC remains live")
                .actor_data()
                .expect("door-pass test PC has actor data")
                .active_door_pass
                .as_ref()
                .expect("translated door pass remains active");
            assert_eq!(pass.current_action, expected_action);
            let remaining_walk_actions = pass
                .steps
                .iter()
                .filter_map(|step| match step {
                    DoorPassStep::Walk { action, .. } => Some(*action),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!remaining_walk_actions.is_empty());
            assert!(
                remaining_walk_actions
                    .iter()
                    .all(|action| *action == expected_action),
                "every remaining translated door rail must retain {expected_action:?}: {remaining_walk_actions:?}"
            );
        }
    }

    #[test]
    fn direct_pc_pass_uses_stamped_moving_fast_sword_state() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(7));
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .action_state,
            crate::element::ActionState::Waiting,
            "the live state deliberately differs from the transition stamp"
        );
        let door = crate::gate::Door {
            door_type: DoorType::Building,
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass_with_transition_state(
            &mut engine,
            &[door],
            owner,
            OrderType::RunningUpright,
            crate::sequence::MoveFlags::FAST,
            Posture::Upright,
            crate::element::ActionState::MovingFastSword,
        );

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        let SequenceElementData::Movement { action, .. } = &element.data else {
            unreachable!()
        };
        assert_eq!(*action, OrderType::RunningWithSword);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::RunningWithSword),
            "the first direct-door rail must retain the stamped fast sword movement"
        );
    }

    #[test]
    fn direct_pc_pass_preserves_authored_run_without_fast_flag_in_sword_state() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(7));
        let door = crate::gate::Door {
            door_type: DoorType::Building,
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass_with_transition_state(
            &mut engine,
            &[door],
            owner,
            OrderType::RunningUpright,
            crate::sequence::MoveFlags::empty(),
            Posture::Upright,
            crate::element::ActionState::MovingSword,
        );

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        let SequenceElementData::Movement { action, flags, .. } = &element.data else {
            unreachable!()
        };
        assert!(flags.is_empty());
        assert_eq!(*action, OrderType::RunningWithSword);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::RunningWithSword),
            "PassDoor must derive sword speed from its authored run, not its empty flags"
        );
    }

    #[test]
    fn direct_stairs_pass_preserves_stamped_crouched_posture() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(7));
        install_lift_sector(&mut engine, LiftType::Stairs);
        let door = crate::gate::Door {
            door_type: DoorType::LiftHigh,
            sector_in: crate::sector::SectorNumber::new(42),
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass_with_transition_state(
            &mut engine,
            &[door],
            owner,
            OrderType::WalkingCrouched,
            crate::sequence::MoveFlags::empty(),
            Posture::Crouched,
            crate::element::ActionState::Moving,
        );

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        let SequenceElementData::Movement { action, .. } = &element.data else {
            unreachable!()
        };
        assert_eq!(*action, OrderType::WalkingCrouched);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::WalkingCrouched),
            "the walk to the stairs midpoint must retain the stamped crouched posture"
        );

        let pass = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_door_pass
            .as_ref()
            .unwrap();
        assert_eq!(pass.current_action, OrderType::WalkingCrouched);
        assert!(pass.steps.iter().any(|step| {
            matches!(
                step,
                DoorPassStep::Walk {
                    action: OrderType::WalkingCrouched,
                    ..
                }
            )
        }));
    }

    #[test]
    fn wall_transition_and_passing_door_use_separate_owner_slots() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(7));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .has_climb = true;
        install_lift_sector(&mut engine, LiftType::Wall);
        let door = crate::gate::Door {
            door_type: DoorType::LiftLow,
            sector_in: crate::sector::SectorNumber::new(42),
            ..default_door()
        };
        engine.script_domains.interactables.doors.push(door.clone());
        let (_, seq_id) = dispatch_pass(&mut engine, &[door], owner);

        let mut door_triggers = Vec::new();
        let mut select_triggers = Vec::new();
        let transition_destination = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .position_map();
        let transition = {
            let actor = engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            EngineInner::advance_door_pass(
                actor,
                owner,
                transition_destination,
                &mut door_triggers,
                &mut select_triggers,
                &mut engine.orders.next_order_id,
            )
        };
        let DoorPassAdvance::Continue {
            destination,
            action,
            reverse,
            compute_direction,
            tolerance,
        } = transition
        else {
            panic!("low direct wall pass must schedule its climb transition: {transition:?}");
        };
        assert_eq!(action, OrderType::TransitionWaitingUprightClimbingWallUp);
        let mut transition_order = crate::order::Order::new(
            action,
            destination.x,
            destination.y,
            engine.orders.allocate_order_id(),
        );
        transition_order.reverse = reverse;
        transition_order.compute_direction = compute_direction;
        transition_order.tolerance = tolerance;
        assert!(door_triggers.is_empty());
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .element_data()
                .sector(),
            crate::position_interface::SectorHandle::new(7)
        );

        engine
            .orders
            .sequence_manager
            .push_order_on(seq_id, 0, transition_order.clone());
        engine.do_next_order(seq_id, 0);

        bind_single_animation(
            &mut engine,
            owner,
            OrderType::TransitionWaitingUprightClimbingWallUp,
        );
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            // The preceding walk step has already delivered the actor to the
            // transition point; a terminated transition short of its goal
            // would instead spawn the Original's distance-continuation copy.
            entity
                .element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(
                    transition_order.target_x,
                    transition_order.target_y,
                ));
            let sprite = &mut entity.element_data_mut().sprite;
            sprite
                .position_iface
                .set_sector(crate::position_interface::SectorHandle::new(7));
            sprite.last_processed_order_id = transition_order.order_id.get();
            sprite.last_action = OrderType::TransitionWaitingUprightClimbingWallUp;
            sprite.current_row = 0;
            sprite.current_frame = 2;
            sprite.frame_count = 9;
            sprite
                .position_iface
                .set_map_goal(crate::coordinates::MapPoint::new(
                    transition_order.target_x,
                    transition_order.target_y,
                ));
            sprite.position_iface.compute_increment_all(false);
            let actor = entity.actor_data_mut().unwrap();
            actor.action_state = crate::element::ActionState::Moving;
        }

        // The terminal transition slot applies its OnWall state and installs
        // PassingDoor, but does not execute the topology callback.
        engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(crate::engine::movement::MovementOwnerSelection {
                seq_id,
                elem_idx: 0,
                order_id: transition_order.order_id,
            }),
        );

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(
            entity.element_data().posture,
            crate::element::Posture::OnWall
        );
        assert_eq!(
            entity.element_data().sector(),
            crate::position_interface::SectorHandle::new(7),
            "transition completion must leave topology on the source side"
        );
        let passing_order = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .clone();
        assert_eq!(passing_order.order_type, OrderType::PassingDoor);
        let position_before = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .element_data()
            .position_map();

        engine.tick_entity_movement_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            owner,
            Some(crate::engine::movement::MovementOwnerSelection {
                seq_id,
                elem_idx: 0,
                order_id: passing_order.order_id,
            }),
        );

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(
            entity.element_data().sector(),
            crate::position_interface::SectorHandle::new(42)
        );
        assert_eq!(entity.element_data().position_map(), position_before);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::ClimbingWallUp,
            "PassingDoor's slot may install, but must not execute, the climb successor"
        );
    }

    #[test]
    fn far_side_projection_selection_does_not_change_door_topology() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(62));
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.element_data_mut().set_layer(3);
        }

        let midpoint = MapPoint::new(2276.0, 1136.0);
        engine.finalize_special_move_position_using_projection_sector(
            &LevelAssets::new(),
            owner,
            crate::engine::special_motion::SpecialMovePosition::Map(midpoint),
            2,
            50,
            MapPoint::new(2272.0, 1123.0),
            "test far-side door projection",
        );

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().position_map(), midpoint);
        assert_eq!(entity.element_data().layer(), 3);
        assert_eq!(
            entity.element_data().sector(),
            crate::position_interface::SectorHandle::new(62),
            "projection lookup is not the explicit PassingDoor topology swap"
        );
    }

    #[test]
    fn direct_door_completion_preserves_rail_elevation_without_plane_reprojection() {
        let mut engine = EngineInner::new();
        engine
            .script_domains
            .interactables
            .doors
            .push(default_door());
        let owner = engine.add_entity(make_pc(7));

        let before_map = MapPoint::new(29.0, 30.0);
        let elevation = 93.3318_f32;
        let ground = crate::coordinates::GroundPoint::from_map_and_z(before_map, elevation);
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.position_iface_mut().set_obstacle(
                None,
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 0.0,
                    bz: 0.0,
                    dz: 90.00101,
                }),
            );
            // The obstacle is already installed before the door-rail
            // movement writes its authoritative 3D endpoint.
            entity
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(
                    ground.x, ground.y, elevation,
                ));
        }

        engine.commit_completed_door_pass_position(
            &LevelAssets::new(),
            owner,
            crate::gate::DoorIndex(0),
            true,
        );

        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(
            entity.element_data().position_map(),
            MapPoint::new(30.0, 30.0)
        );
        assert_eq!(
            entity.element_data().position().z.to_bits(),
            elevation.to_bits(),
            "the direct branch must preserve the door-rail Z instead of resolving the plane"
        );
        assert_eq!(
            entity.element_data().position().to_map(),
            entity.element_data().position_map(),
            "the endpoint snap must keep map and world coordinates coherent"
        );
    }

    #[test]
    fn wall_up_transition_completion_recomputes_midpoint_on_installed_rail_plane() {
        use crate::sight_obstacle::{ObstaclePoint, SIGHTOBSTACLE_PROJECTION_AREA, SightObstacle};

        for start in [MapPoint::new(20.0, 30.0), MapPoint::new(10.0, 30.0)] {
            let mut engine = EngineInner::new();
            engine
                .script_domains
                .interactables
                .doors
                .push(default_door());
            let owner = engine.add_entity(make_pc(7));

            // Competing authored projection: resolving the source-sector
            // projection at the midpoint would flatten the actor to Z=90.
            // Original's transition arm does not perform that lookup.
            let mut flat_projection = SightObstacle::new(
                1,
                crate::sight_obstacle::SIGHTOBSTACLE_SOLID | SIGHTOBSTACLE_PROJECTION_AREA,
            );
            flat_projection.layer = 0;
            flat_projection.sector = 7;
            flat_projection.material = 2;
            flat_projection.obstacle_points = vec![
                ObstaclePoint {
                    x: 0.0,
                    y: 110.0,
                    z_bottom: 0.0,
                    z_top: 90.0,
                },
                ObstaclePoint {
                    x: 40.0,
                    y: 110.0,
                    z_bottom: 0.0,
                    z_top: 90.0,
                },
                ObstaclePoint {
                    x: 40.0,
                    y: 130.0,
                    z_bottom: 0.0,
                    z_top: 90.0,
                },
                ObstaclePoint {
                    x: 0.0,
                    y: 130.0,
                    z_bottom: 0.0,
                    z_top: 90.0,
                },
            ];
            flat_projection.top_plane_points =
                [[0.0, 110.0, 90.0], [40.0, 110.0, 90.0], [0.0, 130.0, 90.0]];
            flat_projection.rebuild_geometry();
            let mut installed_rail = SightObstacle::new_default(2);
            installed_rail.material = 1;
            let mut assets = LevelAssets::new();
            assets.static_sight_obstacles =
                std::sync::Arc::new(vec![flat_projection, installed_rail]);
            engine.world.static_sight_obstacle_active = vec![true, true];
            assert_eq!(
                engine.get_projection_area_index(&assets, 7, 0, MapPoint::new(20.0, 30.0)),
                Some(0),
                "the control projection must genuinely compete at the transition midpoint"
            );

            let rail_plane = crate::position_interface::PlaneZCoeffs {
                az: 0.1,
                bz: 0.0,
                dz: 91.3318,
            };
            {
                let entity = engine.world.entities.get_mut(owner).unwrap();
                let pi = entity.position_iface_mut();
                pi.set_obstacle(
                    crate::position_interface::ObstacleHandle::new(1),
                    Some(rail_plane),
                );
                pi.set_material(crate::element::GameMaterial::Wood);
                pi.set_map_position(start);
                pi.set_old_map_position(start);
                pi.set_old_position(pi.get_position());
                entity.actor_data_mut().unwrap().active_door_pass = Some(ActiveDoorPass {
                    door_index: crate::gate::DoorIndex(0),
                    direct: true,
                    position_direct: true,
                    steps: VecDeque::new(),
                    triggers_fired: 0,
                    current_action: OrderType::TransitionWaitingUprightClimbingWallUp,
                    current_reverse: false,
                    saved_action_state: None,
                });
            }

            engine.apply_door_pass_transition_completion_side_effects(&assets, owner);

            let entity = engine.world.entities.get(owner).unwrap();
            let pi = entity.position_iface();
            assert_eq!(pi.map_position(), MapPoint::new(20.0, 30.0));
            assert_eq!(
                pi.get_obstacle(),
                crate::position_interface::ObstacleHandle::new(1)
            );
            assert_eq!(pi.get_plane(), Some(&rail_plane));
            assert_eq!(pi.get_material(), crate::element::GameMaterial::Wood);
            assert_eq!(
                pi.get_elevation().to_bits(),
                93.3318_f32.to_bits(),
                "SetPositionMap + ComputePositionAll must project the midpoint on the installed rail plane"
            );
            if start == MapPoint::new(20.0, 30.0) {
                assert!(
                    !pi.is_moving(),
                    "an in-place transition completion must not publish phantom 3D movement"
                );
            }
        }
    }

    #[test]
    fn direct_door_completion_does_not_reconstruct_an_already_committed_endpoint() {
        let mut engine = EngineInner::new();
        let endpoint = MapPoint::new(250.0, 270.0);
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_in: endpoint,
                ..default_door()
            });
        let owner = engine.add_entity(make_pc(7));

        // At this magnitude, `(map_y + z) - z` is one ULP below map_y.
        // That is exactly why C++ direct PassDoor leaves the final rail
        // position alone instead of recomputing it.
        let elevation = 244.55569458007812_f32;
        let ground = crate::coordinates::GroundPoint::from_map_and_z(endpoint, elevation);
        let world = crate::coordinates::WorldPoint3D::new(ground.x, ground.y, elevation);
        assert_ne!(world.to_map().y.to_bits(), endpoint.y.to_bits());
        {
            let entity = engine.world.entities.get_mut(owner).unwrap();
            entity.element_data_mut().set_position(world);
            // Preserve the independently committed C++-style world
            // representation; map XY is authoritative for movement parity.
            entity
                .element_data_mut()
                .set_position_map_preserving_3d(endpoint);
        }
        let before = engine.get_entity(owner).unwrap().element_data().position();

        engine.commit_completed_door_pass_position(
            &LevelAssets::new(),
            owner,
            crate::gate::DoorIndex(0),
            true,
        );

        let entity = engine.get_entity(owner).unwrap();
        assert_eq!(entity.element_data().position_map(), endpoint);
        assert_eq!(entity.element_data().position(), before);
    }

    #[test]
    fn denied_door_disables_anti_collision_before_marking_impossible() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(7));
        let door = crate::gate::Door {
            locked_pc: true,
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

        assert_eq!(barrier, PassDoorLaunchBarrier::SkipSplice);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Impossible
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .orders
                .is_empty()
        );
        assert!(
            !engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .position_iface()
                .is_anti_collision_on(),
            "RHelementactor.cpp disables anti-collision before authorization"
        );
    }

    #[test]
    fn wall_lift_rejects_soldier_before_installing_an_order() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(Some(7)));
        let lift_sector = crate::sector::SectorNumber::new(42);
        install_lift_sector(&mut engine, LiftType::Wall);
        let door = crate::gate::Door {
            door_type: DoorType::LiftHigh,
            sector_in: lift_sector,
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

        assert_eq!(barrier, PassDoorLaunchBarrier::SkipSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::Impossible);
        assert!(element.orders.is_empty());
        assert!(
            !engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .position_iface()
                .is_anti_collision_on()
        );
    }

    #[test]
    fn ladder_lift_uses_ladder_translation_and_reaches_splice() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(Some(7)));
        let lift_sector = crate::sector::SectorNumber::new(42);
        install_lift_sector(&mut engine, LiftType::Ladder);
        let door = crate::gate::Door {
            door_type: DoorType::LiftHigh,
            sector_in: lift_sector,
            ..default_door()
        };

        let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);

        assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        let initial_walk = element.current_order().unwrap();
        assert_eq!(initial_walk.order_type, OrderType::WalkingUpright);
        assert!(initial_walk.reverse);
        let pass = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_door_pass
            .as_ref()
            .unwrap();
        assert!(matches!(
            pass.steps.front(),
            Some(DoorPassStep::Transition {
                action: OrderType::Turning,
                reverse: true,
            })
        ));
    }

    #[test]
    fn building_trap_decorative_ladder_down_preserves_facing_without_lift_metadata() {
        for frozen in [false, true] {
            let mut engine = EngineInner::new();
            let owner = engine.add_entity(make_soldier(Some(7)));
            let door = crate::gate::Door {
                door_type: DoorType::BuildingTrap,
                ..default_door()
            };
            engine.script_domains.interactables.doors.push(door.clone());

            let (barrier, seq_id) = dispatch_pass(&mut engine, &[door], owner);
            assert_eq!(barrier, PassDoorLaunchBarrier::ReachSplice);
            assert!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .actor_data()
                    .unwrap()
                    .active_door_pass
                    .as_ref()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|step| matches!(
                        step,
                        DoorPassStep::Walk {
                            action: OrderType::ClimbingLadderDown,
                            reverse: true,
                            ..
                        }
                    )),
                "the production BuildingTrap chain must supply its decorative reverse ladder row"
            );

            bind_single_animation(&mut engine, owner, OrderType::ClimbingLadderDown);
            let order_id = engine.orders.allocate_order_id();
            {
                let element = engine
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, 0)
                    .unwrap();
                element.orders.clear();
                let mut order =
                    crate::order::Order::new(OrderType::ClimbingLadderDown, 30.0, 30.0, order_id);
                order.reverse = true;
                order.compute_direction = false;
                element.orders.push_back(order);
            }
            {
                let entity = engine.get_entity_mut(owner).unwrap();
                entity
                    .element_data_mut()
                    .set_position_map(MapPoint::new(30.0, 30.0));
                entity
                    .element_data_mut()
                    .set_sector(crate::position_interface::SectorHandle::new(8));
                entity.element_data_mut().set_direction_instantly(7);
                entity.element_data_mut().set_direction_goal(9);
                let actor = entity.actor_data_mut().unwrap();
                actor.execute_order_initialising = true;
                actor.active_door_pass.as_mut().unwrap().current_action =
                    OrderType::ClimbingLadderDown;
                actor.active_door_pass.as_mut().unwrap().current_reverse = true;
            }
            engine.set_actors_frozen(frozen);

            engine.tick_entity_movement_owner(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                Some(crate::engine::movement::MovementOwnerSelection {
                    seq_id,
                    elem_idx: 0,
                    order_id,
                }),
            );

            assert_eq!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .position_iface()
                    .get_direction_goal()
                    .as_u8(),
                9,
                "a decorative BuildingTrap ladder row must not read or replace lift facing (frozen={frozen})"
            );
            if frozen {
                assert_eq!(
                    engine.get_entity(owner).unwrap().element_data().direction(),
                    8,
                    "FrozenAll suppresses sprite motion, not the decorative climb arm's ordinary Turn()"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "has no sector for door 0 direction resolution")]
    fn missing_actor_sector_is_an_invariant_failure() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(None));

        let _ = dispatch_pass(&mut engine, &[default_door()], owner);
    }

    #[test]
    #[should_panic(expected = "is on neither side of door 0")]
    fn actor_sector_must_match_one_side_of_the_door() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(Some(99)));

        let _ = dispatch_pass(&mut engine, &[default_door()], owner);
    }

    #[test]
    #[should_panic(expected = "is a lift door but sector 8 has no lift type")]
    fn lift_door_requires_canonical_lift_type() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(Some(7)));
        let door = crate::gate::Door {
            door_type: DoorType::LiftHigh,
            ..default_door()
        };

        let _ = dispatch_pass(&mut engine, &[door], owner);
    }

    #[test]
    fn production_lift_callbacks_and_transition_turn_without_snapping_in_swapped_creation_order() {
        for (lift_type, action, expected_direction) in [
            (
                LiftType::Ladder,
                OrderType::TransitionWaitingUprightClimbingLadderUp,
                1,
            ),
            (LiftType::Ladder, OrderType::ClimbingLadderUpFast, 2),
            (
                LiftType::Wall,
                OrderType::TransitionWaitingUprightClimbingWallUp,
                1,
            ),
            (LiftType::Wall, OrderType::ClimbingWallUpFast, 2),
        ] {
            for owner_is_earlier in [true, false] {
                let mut engine = EngineInner::new();
                let owner = if owner_is_earlier {
                    let owner = engine.add_entity(make_pc(7));
                    let _observer = engine.add_entity(make_soldier(Some(7)));
                    owner
                } else {
                    let _observer = engine.add_entity(make_soldier(Some(7)));
                    engine.add_entity(make_pc(7))
                };
                engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .pc_data_mut()
                    .unwrap()
                    .has_climb = true;
                install_lift_sector(&mut engine, lift_type);
                let door = crate::gate::Door {
                    door_type: DoorType::LiftLow,
                    sector_in: crate::sector::SectorNumber::new(42),
                    ..default_door()
                };
                engine.script_domains.interactables.doors.push(door.clone());
                let (_, seq_id) = dispatch_pass(&mut engine, &[door], owner);
                bind_single_animation(&mut engine, owner, action);

                let elem_idx = 0;
                let order_id = {
                    let order = engine
                        .orders
                        .sequence_manager
                        .get_element_mut(seq_id, elem_idx)
                        .unwrap()
                        .orders
                        .front_mut()
                        .unwrap();
                    order.order_type = action;
                    order.compute_direction = false;
                    order.target_x = 20.0;
                    order.target_y = 30.0;
                    order.order_id
                };
                {
                    let entity = engine.world.entities.get_mut(owner).unwrap();
                    entity.element_data_mut().set_direction_instantly(i16::from(
                        crate::position_interface::Direction::NORTH,
                    ));
                    entity
                        .element_data_mut()
                        .set_sector(crate::position_interface::SectorHandle::new(7));
                    let actor = entity.actor_data_mut().unwrap();
                    actor.action_state = crate::element::ActionState::Waiting;
                    actor.execute_order_initialising = true;
                    actor.active_door_pass.as_mut().unwrap().current_action = action;
                }

                engine.execute_pass_door(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::new(),
                    owner,
                    crate::gate::DoorIndex(0),
                    true,
                    0,
                );
                assert_eq!(
                    engine
                        .world
                        .entities
                        .get(owner)
                        .unwrap()
                        .element_data()
                        .direction(),
                    0,
                    "the PassingDoor action point changes sector but must not snap lift facing"
                );

                engine.tick_entity_movement_owner(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::new(),
                    owner,
                    Some(crate::engine::movement::MovementOwnerSelection {
                        seq_id,
                        elem_idx,
                        order_id,
                    }),
                );

                let entity = engine.world.entities.get(owner).unwrap();
                assert_eq!(
                    entity.element_data().direction(),
                    expected_direction,
                    "fast climb Execute must run its two original Turn() iterations"
                );
                assert_eq!(
                    entity.position_iface().get_direction_goal().as_u8(),
                    5,
                    "{lift_type:?} transition must use the inside lift sector's direction goal"
                );
                engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .element_data_mut()
                    .set_direction_goal(7);
                engine.apply_completed_door_pass_lift_entry_state(
                    owner,
                    crate::gate::DoorIndex(0),
                    true,
                );
                assert_eq!(
                    engine
                        .world
                        .entities
                        .get(owner)
                        .unwrap()
                        .element_data()
                        .direction(),
                    expected_direction,
                    "door-pass completion must preserve the gradual Turn() result"
                );
                assert_eq!(
                    engine
                        .world
                        .entities
                        .get(owner)
                        .unwrap()
                        .position_iface()
                        .get_direction_goal()
                        .as_u8(),
                    7,
                    "PassingDoor changes lift topology but must not set a new facing goal"
                );
            }
        }
    }

    #[test]
    fn frozen_all_climbs_turn_in_owner_slot_with_real_swapped_owner_visibility() {
        for (action, expected_direction) in [
            (OrderType::TransitionWaitingUprightClimbingLadderUp, 1),
            (OrderType::ClimbingLadderUpFast, 2),
        ] {
            for climber_is_earlier in [true, false] {
                let mut engine = EngineInner::new();
                let (climber, observer) = if climber_is_earlier {
                    (engine.add_entity(make_pc(7)), engine.add_entity(make_pc(7)))
                } else {
                    let observer = engine.add_entity(make_pc(7));
                    let climber = engine.add_entity(make_pc(7));
                    (climber, observer)
                };
                let _ = install_production_climb_fixture(
                    &mut engine,
                    climber,
                    LiftType::Ladder,
                    action,
                );
                engine.set_actors_frozen(true);

                let positions =
                    crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
                let mut direction_seen_by_observer = None;
                engine.tick_actor_owner_envelopes_with_test_owner_hook(
                    &crate::sim_rng::test_context(),
                    &LevelAssets::new(),
                    &positions,
                    |engine, completed_owner| {
                        if completed_owner == observer {
                            direction_seen_by_observer = Some(
                                engine
                                    .world
                                    .entities
                                    .get(climber)
                                    .unwrap()
                                    .element_data()
                                    .direction(),
                            );
                        }
                    },
                );

                assert_eq!(
                    engine
                        .world
                        .entities
                        .get(climber)
                        .unwrap()
                        .element_data()
                        .direction(),
                    expected_direction,
                    "FrozenAll suppresses sprite motion, not climb Execute Turn()"
                );
                assert_eq!(
                    direction_seen_by_observer,
                    Some(if climber_is_earlier {
                        expected_direction
                    } else {
                        0
                    }),
                    "only the genuinely later owner slot may see the frozen climb turn"
                );
            }
        }
    }

    #[test]
    fn fast_climb_first_iteration_termination_prevents_second_turn_in_swapped_creation_order() {
        for owner_is_earlier in [true, false] {
            let mut engine = EngineInner::new();
            let owner = if owner_is_earlier {
                let owner = engine.add_entity(make_pc(7));
                let _observer = engine.add_entity(make_pc(7));
                owner
            } else {
                let _observer = engine.add_entity(make_pc(7));
                engine.add_entity(make_pc(7))
            };
            let (seq_id, order_id) = install_production_climb_fixture(
                &mut engine,
                owner,
                LiftType::Wall,
                OrderType::ClimbingWallUpFast,
            );
            {
                let element = engine
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap()
                    .element_data_mut();
                // Walk/Run motion terminates only on map arrival, never on
                // the animation loop. Give the climb frames real distance and
                // start within one projected step of the goal so the first
                // PerformMotion pair reaches it.
                let mut conversion = vec![
                    crate::sprite_script::UNMAPPED;
                    crate::sprite_script::NONANIMATION_END
                        .max(OrderType::ClimbingWallUpFast as usize + 1)
                ];
                conversion[OrderType::ClimbingWallUpFast as usize] = 0;
                conversion[OrderType::ClimbingWallUp as usize] = 0;
                let script = crate::sprite_script::SpriteScript {
                    action_id: OrderType::ClimbingWallUpFast as u16,
                    action_done: 1,
                    average_speed: 10.0,
                    hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
                    sum_distance: 30,
                    frame_ids: vec![1, 2, 3],
                    delays: vec![10, 10, 10],
                    distances: vec![10, 10, 10],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                    sound_ids: vec![0, 0, 0],
                };
                element.sprite = crate::sprite::Sprite::new(
                    std::sync::Arc::new(std::iter::repeat_n(script, 16).collect()),
                    std::sync::Arc::new(conversion),
                );
                element.set_position_map(crate::coordinates::MapPoint::new(20.0, 26.0));
                let sprite = &mut element.sprite;
                sprite
                    .position_iface
                    .set_sector(crate::position_interface::SectorHandle::new(7));
                // Door-pass translation disables anti-collision for the
                // duration of the pass.
                sprite.position_iface.set_anti_collision_on(false);
                sprite.last_processed_order_id = order_id.get();
                sprite.last_action = OrderType::ClimbingWallUp;
                sprite.current_row = 0;
                sprite.current_frame = 2;
                sprite.frame_count = 10;
                sprite
                    .position_iface
                    .set_map_goal(crate::coordinates::MapPoint::new(20.0, 30.0));
                sprite.position_iface.compute_increment_all(false);
            }
            // The skipped initialising Execute stamped the lift direction as
            // the retained turn goal; only the per-tick Turn steps remain.
            engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .element_data_mut()
                .set_direction_goal(5);

            engine.tick_entity_movement_owner(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                owner,
                Some(crate::engine::movement::MovementOwnerSelection {
                    seq_id,
                    elem_idx: 0,
                    order_id,
                }),
            );

            assert_eq!(
                engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap()
                    .element_data()
                    .direction(),
                1,
                "first fast PerformMotion termination must skip the second Turn/PerformMotion pair"
            );
        }
    }

    #[test]
    #[should_panic(expected = "is not movement data")]
    fn non_movement_pass_door_is_an_invariant_failure() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(Some(7)));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(
                1,
                crate::element::Command::PassDoor,
                Some(owner),
            ));

        PassDoorLaunchContext::new(
            &[default_door()],
            &mut engine.world.entities,
            &engine.world.fast_grid,
            &mut engine.orders.sequence_manager,
            &mut engine.orders.next_order_id,
        )
        .dispatch(owner, seq_id, 0);
    }
}
