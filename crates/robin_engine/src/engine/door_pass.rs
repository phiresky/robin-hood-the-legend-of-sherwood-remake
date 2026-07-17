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

/// Return value from [`EngineInner::build_door_pass`].
///
/// Pairs the built step chain with a post-install action-recursive
/// override.  When the PC exits a ladder/wall pass into a forced-crouch
/// sector, the element's root action must be rewritten to
/// `WalkingCrouched`.  The caller applies the override via
/// `SequenceManager::set_action_recursive` after the PassDoor element
/// is installed so the element's root action reads WalkingCrouched
/// instead of the upstream-chosen `ctx.action`.
pub(super) struct BuiltDoorPass {
    pub pass: ActiveDoorPass,
    pub post_chain_action_recursive: Option<OrderType>,
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

impl EngineInner {
    /// Build the complete door-pass step chain for the given door and actor.
    ///
    /// Dispatches to the appropriate translate function based on door type
    /// and lift type.
    pub(super) fn build_door_pass(
        &mut self,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
        flags: crate::sequence::MoveFlags,
    ) -> Option<BuiltDoorPass> {
        // Snapshot door geometry and type (releases borrow on mission_script).
        let (door_type, pt_mid, pt_in, pt_out, sector_in, door_sector_out) = {
            let game_host = self.mission_script.as_mut()?.game_host_mut()?;
            let door = game_host.doors.get(usize::from(door_index))?;
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
        let entity = self.get_entity(entity_id)?;
        let is_pc = entity.is_pc();
        let is_soldier = entity.is_soldier();
        let posture = entity.element_data().posture;
        let action_state = entity.actor_data().map(|a| a.action_state);
        let is_carrying = posture == Posture::CarryingOnShoulders;

        // Soldier attentive state: true while in a sword/shield action
        // state.  Attentive soldiers use different ladder/wall climb
        // transition animations.
        let is_attentive = is_soldier
            && matches!(
                action_state,
                Some(crate::element::ActionState::WaitingSword)
                    | Some(crate::element::ActionState::MovingSword)
                    | Some(crate::element::ActionState::HoldingShield)
                    | Some(crate::element::ActionState::MovingShield)
                    | Some(crate::element::ActionState::Menacing)
            );

        // Choose base movement animation.
        // Door-pass uses the `WalkingWith*` / `RunningWith*` variants so
        // the stairs translator routes them through the sword/shield
        // branch instead of the plain walk/run branch.
        let is_fast = flags.contains(crate::sequence::MoveFlags::FAST);
        let action = if is_carrying {
            OrderType::WalkingCarryingOnShoulders
        } else if matches!(
            action_state,
            Some(crate::element::ActionState::WaitingSword)
                | Some(crate::element::ActionState::MovingSword)
                | Some(crate::element::ActionState::ParryingSword)
                | Some(crate::element::ActionState::ParryingSwordLow)
                | Some(crate::element::ActionState::Menacing)
        ) {
            if is_fast {
                OrderType::RunningWithSword
            } else {
                OrderType::WalkingWithSword
            }
        } else if matches!(
            action_state,
            Some(crate::element::ActionState::HoldingShield)
                | Some(crate::element::ActionState::MovingShield)
        ) {
            // No running-with-shield variant — shield posture is
            // always a walk regardless of the fast flag.
            OrderType::WalkingWithShield
        } else {
            OrderType::WalkingUpright
        };

        let sector_out_forces_crouch = self.sector_forces_crouch(door_sector_out);
        let sector_in_forces_crouch = self.sector_forces_crouch(sector_in);

        // Determine lift type for lift doors.
        let lift_type = match door_type {
            DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::LiftLow => {
                self.get_sector_lift_type(sector_in)
            }
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
        let sprite = self.get_entity(entity_id).map(|e| e.sprite());
        let dist = |anim: OrderType| -> f32 {
            sprite
                .map(|s| f32::from(s.distance_for_animation(anim)))
                .unwrap_or(0.0)
        };
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
                None => translate_stairs(&ctx),
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

        Some(BuiltDoorPass {
            pass: ActiveDoorPass {
                door_index,
                direct,
                steps,
                triggers_fired: 0,
                current_action: action,
                current_reverse: false,
                saved_action_state: None,
            },
            post_chain_action_recursive,
        })
    }

    /// Execute the PassDoor callback — change layer/sector and trigger
    /// building/lift callbacks.
    ///
    /// Called when a [`DoorPassStep::PassingDoor`] step fires.
    /// First call (trigger 0) changes layer/sector; subsequent calls
    /// re-enable anti-collision.
    pub(super) fn execute_pass_door(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        _direct: bool,
        trigger_number: u8,
    ) {
        if trigger_number > 0 {
            // Second (and later) trigger:
            // RHNONANIMATION_PASSING_DOOR calls SetAntiCollisionOn(true)
            // once PassDoor() has already consumed the gate.
            if let Some(entity) = self.get_entity_mut(entity_id) {
                entity.position_iface_mut().set_anti_collision_on(true);
            }
            return;
        }

        // ── First trigger: perform the layer/sector change ──

        // Snapshot door data before mutable borrows.
        let (target_layer, target_sector_num, _door_type, _is_lift_high, door_point_out) = {
            let game_host = match self.mission_script.as_mut().and_then(|s| s.game_host_mut()) {
                Some(h) => h,
                None => return,
            };
            let door = match game_host.doors.get(usize::from(door_index)) {
                Some(d) => d,
                None => return,
            };
            let (tl, ts) = if _direct {
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
        let (current_sector, is_pc) = match self.get_entity(entity_id) {
            Some(e) => (e.element_data().sector(), e.is_pc()),
            None => return,
        };
        let actor_handle = crate::natives::GameHost::actor_handle(entity_id);

        // ── Leave callbacks ──
        // Track whether we're leaving a building so we can refresh the
        // actor's projection-area obstacle + footstep material after the
        // layer/sector change: on building exit we re-seat the actor
        // onto the projection area at the door's outside point so the
        // next footstep sounds use the correct material.
        let mut left_building = false;
        if let Some(cur_sector_handle) = current_sector {
            let cur_sector_num: u16 = cur_sector_handle.into();
            let gs =
                self.grid_sector_by_number(crate::sector::SectorNumber::new(cur_sector_num as i16));

            if gs.map(|s| s.sector_type.is_building()).unwrap_or(false) {
                left_building = true;
                // Leaving a building — remove from occupant list.
                let bld_idx = gs.and_then(|s| s.building_index);
                if let Some(bi) = bld_idx
                    && let Some(ref mut script) = self.mission_script
                    && let Some(game_host) = script.game_host_mut()
                {
                    if let Some(occupants) = game_host.building_occupants.get_mut(usize::from(bi)) {
                        occupants.retain(|&a| a != actor_handle);
                    }
                    game_host.actor_building.remove(&actor_handle);
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
                    entity.element_data_mut().hidden_in_building = false;
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
                    carried.element_data_mut().hidden_in_building = false;
                }
                // When the leaving actor is a PC, (a) recursively
                // remove its carried actor (mirrors the Enter-side
                // push), and (b) if no PC remains in the building,
                // hide every other currently-visible occupant — the
                // corpses that the Enter side had unhidden go back
                // into "stored" state.
                if is_pc && let Some(bi) = bld_idx {
                    if let Some(carried_id) = carried_to_unhide {
                        let carried_h = crate::natives::GameHost::actor_handle(carried_id);
                        if let Some(ref mut script) = self.mission_script
                            && let Some(game_host) = script.game_host_mut()
                        {
                            if let Some(occupants) =
                                game_host.building_occupants.get_mut(usize::from(bi))
                            {
                                occupants.retain(|&a| a != carried_h);
                            }
                            game_host.actor_building.remove(&carried_h);
                        }
                    }
                    // Snapshot the post-removal occupant list so we can
                    // probe each occupant without holding the script borrow.
                    let occupants: Vec<i32> = self
                        .mission_script
                        .as_ref()
                        .and_then(|s| s.game_host())
                        .and_then(|gh| gh.building_occupants.get(usize::from(bi)))
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
            } else if gs.map(|s| s.sector_type.is_lift()).unwrap_or(false) {
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
                    let st = self.world.fast_grid.lift_state_mut(grid_idx as u32);
                    st.occupants = st.occupants.saturating_sub(1);
                    if st.occupants == 0 {
                        st.occupied_upwards = false;
                        st.occupied_downwards = false;
                        st.wait_time = 0;
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
        // Look up the destination sector's stored facing once so we can
        // apply it after the layer/sector change for lift sectors:
        // every climbing-ladder / climbing-wall animation starts by
        // snapping the actor's direction to the sector's stored
        // direction.
        let lift_facing = self
            .grid_sector_by_number(target_sector_num)
            .filter(|gs| gs.sector_type.is_lift())
            .map(|gs| gs.lift_direction);
        if let Some(entity) = self.get_entity_mut(entity_id) {
            let elem = entity.element_data_mut();
            elem.set_layer(target_layer);
            elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
                target_sector_num,
            )));
            if let Some(dir) = lift_facing {
                elem.set_direction_instantly(dir);
            }
            tracing::debug!(
                entity_id = ?entity_id,
                layer = target_layer,
                sector = %target_sector_num,
                "PassDoor: changed layer/sector"
            );
        }

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
        let enter_gs = self.grid_sector_by_number(target_sector_num);
        if enter_gs
            .map(|s| s.sector_type.is_building())
            .unwrap_or(false)
        {
            // Entering a building — add to occupant list.
            let bld_idx = enter_gs.and_then(|s| s.building_index);
            if let Some(bi) = bld_idx {
                let bld_handle =
                    crate::natives::GameHost::building_handle_from_index(usize::from(bi));
                if let Some(ref mut script) = self.mission_script
                    && let Some(game_host) = script.game_host_mut()
                {
                    if usize::from(bi) >= game_host.building_occupants.len() {
                        game_host
                            .building_occupants
                            .resize(usize::from(bi) + 1, Vec::new());
                    }
                    game_host.building_occupants[usize::from(bi)].push(actor_handle);
                    game_host.actor_building.insert(actor_handle, bld_handle);
                }
            }
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
                entity.element_data_mut().hidden_in_building = true;
                // Special case: a PC carrying a corpse drags the body
                // into the building too — also hidden.
                entity.pc_data().and_then(|pc| pc.carried)
            } else {
                None
            };
            if let Some(carried_id) = carried_to_hide
                && let Some(carried) = self.get_entity_mut(carried_id)
            {
                carried.element_data_mut().hidden_in_building = true;
            }
            // When the entering actor is a PC, (a) recursively enter
            // its carried actor — which adds it to the occupant list —
            // and (b) re-enable existing occupants who are dead /
            // unconscious and not being carried so their corpses render
            // to the freshly-arrived PC.  Matches the script-side
            // `PutActorInBuilding` helper.
            if is_pc && let Some(bi) = bld_idx {
                if let Some(carried_id) = carried_to_hide {
                    let carried_h = crate::natives::GameHost::actor_handle(carried_id);
                    let bld_handle =
                        crate::natives::GameHost::building_handle_from_index(usize::from(bi));
                    if let Some(ref mut script) = self.mission_script
                        && let Some(game_host) = script.game_host_mut()
                    {
                        if usize::from(bi) >= game_host.building_occupants.len() {
                            game_host
                                .building_occupants
                                .resize(usize::from(bi) + 1, Vec::new());
                        }
                        game_host.building_occupants[usize::from(bi)].push(carried_h);
                        game_host.actor_building.insert(carried_h, bld_handle);
                    }
                }
                // Re-enable corpses already inside the building: walk
                // the occupant list and unhide humans that are
                // (dead || unconscious) && not currently carried.
                let occupants: Vec<i32> = self
                    .mission_script
                    .as_ref()
                    .and_then(|s| s.game_host())
                    .and_then(|gh| gh.building_occupants.get(usize::from(bi)))
                    .cloned()
                    .unwrap_or_default();
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
                        occ.element_data_mut().hidden_in_building = false;
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
        self.apply_door_patch(assets, door_index);

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

    pub(super) fn apply_completed_door_pass_lift_entry_state(
        &mut self,
        entity_id: EntityId,
        door_index: crate::gate::DoorIndex,
        direct: bool,
    ) {
        let Some((target_sector, lift_type, lift_direction)) = (|| {
            let game_host = self.mission_script.as_ref()?.game_host()?;
            let door = game_host.doors.get(usize::from(door_index))?;
            let target_sector = if direct {
                door.sector_in
            } else {
                door.sector_out
            };
            let sector = self.grid_sector_by_number(crate::sector::SectorNumber::new(
                i16::from(target_sector),
            ))?;
            Some((target_sector, sector.lift_type?, sector.lift_direction))
        })() else {
            return;
        };

        let posture = match lift_type {
            LiftType::Wall => crate::element::Posture::OnWall,
            LiftType::Ladder => crate::element::Posture::OnLadder,
            _ => return,
        };

        let Some(entity) = self.get_entity_mut(entity_id) else {
            return;
        };
        entity.set_posture(posture);
        entity
            .element_data_mut()
            .set_direction_instantly(lift_direction);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = crate::element::ActionState::Moving;
        }
        tracing::debug!(
            entity = ?entity_id,
            sector = %target_sector,
            ?posture,
            lift_direction,
            "DoorPass: completed into lift, preserving climb idle state"
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
    fn apply_door_patch(&mut self, assets: &LevelAssets, door_index: crate::gate::DoorIndex) {
        // Snapshot the patch_index from the door (avoid overlapping borrows).
        let patch_index = {
            let game_host = match self.mission_script.as_mut().and_then(|s| s.game_host_mut()) {
                Some(h) => h,
                None => return,
            };
            match game_host.doors.get(usize::from(door_index)) {
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
        // patch's FX animation ends (see `tick_entity_animations`).
        let was_applied = {
            let game_host = match self.mission_script.as_ref().and_then(|s| s.game_host()) {
                Some(h) => h,
                None => return,
            };
            match game_host.patches.get(patch_idx) {
                Some(p) => p.applied,
                None => return,
            }
        };
        if let Some(game_host) = self.mission_script.as_mut().and_then(|s| s.game_host_mut())
            && let Some(door) = game_host.doors.get_mut(usize::from(door_index))
        {
            if was_applied {
                door.gate_state.request_close();
            } else {
                door.gate_state.request_open();
            }
        }

        // Apply the patch and collect effects.
        let effects = {
            let game_host = match self.mission_script.as_mut().and_then(|s| s.game_host_mut()) {
                Some(h) => h,
                None => return,
            };
            let patch = match game_host.patches.get_mut(patch_idx) {
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

        self.process_patch_effects(assets, patch_index, effects);
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
