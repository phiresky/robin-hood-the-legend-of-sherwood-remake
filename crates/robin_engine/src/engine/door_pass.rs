//! Door-pass translation and execution.
//!
//! Appends the ordinary orders for door traversal and
//! executes the layer/sector swap when the actor crosses the door.

use super::*;
use crate::coordinates::MapPoint;
use crate::element::{ActiveDoorPass, EntityId, Posture};
use crate::gate::DoorType;
use crate::order::OrderType;
use crate::sector::LiftType;

mod steps;
mod transitions;
pub(super) use steps::*;

// ─── Ordinary order construction ──────────────────────────────────────

/// Appends each translated door order immediately to its owning element.
struct DoorOrders<'a> {
    element: &'a mut crate::sequence::SequenceElement,
    next_id: &'a mut u32,
}

impl DoorOrders<'_> {
    fn push(
        &mut self,
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        tolerance: f32,
    ) {
        let mut order = crate::order::Order::new(
            action,
            destination.x,
            destination.y,
            crate::order::alloc_order_id(self.next_id),
        );
        order.reverse = reverse;
        order.compute_direction = compute_direction;
        order.tolerance = tolerance;
        self.element.push_order(order);
    }

    fn walk(&mut self, destination: MapPoint, action: OrderType) {
        self.push(destination, action, false, true, 0.0);
    }
    fn walk_tol(&mut self, destination: MapPoint, action: OrderType, tolerance: f32) {
        self.push(destination, action, false, true, tolerance);
    }
    fn walk_rev_tol(&mut self, destination: MapPoint, action: OrderType, tolerance: f32) {
        self.push(destination, action, true, true, tolerance);
    }
    fn walk_nodir(&mut self, destination: MapPoint, action: OrderType) {
        self.push(destination, action, false, false, 0.0);
    }
    fn walk_nodir_tol(&mut self, destination: MapPoint, action: OrderType, tolerance: f32) {
        self.push(destination, action, false, false, tolerance);
    }
    fn walk_rev_nodir(&mut self, destination: MapPoint, action: OrderType) {
        self.push(destination, action, true, false, 0.0);
    }
    fn transition(&mut self, action: OrderType) {
        self.push(MapPoint::ZERO, action, false, true, 0.0);
    }
    fn transition_rev(&mut self, action: OrderType) {
        self.push(MapPoint::ZERO, action, true, true, 0.0);
    }
    fn select(&mut self, speed: f32) {
        self.push(MapPoint::ZERO, OrderType::Select, false, true, speed);
    }
    fn passing_door(&mut self) {
        self.push(MapPoint::ZERO, OrderType::PassingDoor, false, true, 0.0);
    }
}

// ─── Door translation inputs ───────────────────────────────

/// Door geometry and actor properties sampled at instruction entry.
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

fn translate_building(ctx: &DoorPassContext, s: &mut DoorOrders<'_>) {
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
        s.walk(ctx.point_mid, action);
        if ctx.is_pc {
            s.select(select_speed(ctx.point_mid, ctx.point_out));
        }
        s.passing_door();
        s.push(ctx.point_in, action, false, false, 0.0);
        // Building-trap: reverse ladder-down animation after entering
        if ctx.door_type == DoorType::BuildingTrap {
            s.push(ctx.point_in, OrderType::ClimbingLadderDown, true, true, 0.0);
        }
        s.passing_door();
    } else {
        // Inside -> outside
        s.push(ctx.point_mid, action, false, false, 0.0);
        if ctx.is_pc {
            s.select(select_speed(ctx.point_mid, ctx.point_in));
        }
        s.passing_door();
        s.walk(ctx.point_out, action);
        s.passing_door();
    }
}

// ─── Ladder door translation ────────────────────────────────────────

/// Walk-step tolerance applied to the walk-to-mid step of the
/// high/non-direct ladder pass so the climb-up transition lands the
/// actor at the ladder's rung rail instead of overshooting the exact
/// point.
const TELEPORT_LADDER: f32 = 45.0;

fn translate_ladder(ctx: &DoorPassContext, s: &mut DoorOrders<'_>) {
    if ctx.is_high {
        if ctx.direct {
            // High, outside -> inside (climb DOWN the ladder).
            // The first walk-to-mid step has a tolerance equal to the
            // climb-down transition distance (plus crouching-down for
            // non-soldier PCs); precomputed in `build_door_pass` via
            // `Sprite::distance_for_animation`.
            s.walk_rev_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_ladder_high_direct,
            );
            s.transition_rev(OrderType::Turning);
            if ctx.is_pc {
                s.transition_rev(OrderType::TransitionCrouchingDown);
            }
            let climb_start = if ctx.is_soldier_attentive {
                OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
            } else {
                OrderType::TransitionWaitingCrouchedClimbingLadderDown
            };
            s.walk_rev_nodir(ctx.point_mid, climb_start);
            s.passing_door();
            s.walk_rev_nodir(ctx.point_in, OrderType::ClimbingLadderDown);
        } else {
            // High, inside -> outside (climb UP the ladder).
            // `TELEPORT_LADDER` (45.0) is set as tolerance on the first
            // walk-to-mid step so the climb-up animation (which already
            // moves the actor past the midpoint) ends before the
            // waypoint is exactly reached.
            s.walk_nodir_tol(ctx.point_mid, OrderType::ClimbingLadderUp, TELEPORT_LADDER);
            let climb_end = if ctx.is_soldier_attentive {
                OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            } else {
                OrderType::TransitionClimbingLadderUpWaitingCrouched
            };
            s.walk_nodir(ctx.point_mid, climb_end);
            if ctx.is_pc && !ctx.sector_out_forces_crouch {
                s.transition(OrderType::TransitionCrouchingUp);
            }
            s.passing_door();
            let exit_action = if ctx.is_pc && ctx.sector_out_forces_crouch {
                OrderType::WalkingCrouched
            } else {
                OrderType::WalkingUpright
            };
            s.walk(ctx.point_out, exit_action);
            s.passing_door();
        }
    } else {
        if ctx.direct {
            // Low, outside -> inside (climb UP).  Tolerance is the
            // `TransitionWaitingUprightClimbingLadderUp` animation
            // distance, precomputed in `build_door_pass` and threaded
            // as `ctx.tol_ladder_low_direct`.
            s.walk_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_ladder_low_direct,
            );
            let climb_start = if ctx.is_soldier_attentive {
                OrderType::TransitionWaitingUprightClimbingLadderUpAlerted
            } else {
                OrderType::TransitionWaitingUprightClimbingLadderUp
            };
            s.walk_nodir(ctx.point_mid, climb_start);
            s.passing_door();
            s.walk_nodir(ctx.point_in, OrderType::ClimbingLadderUp);
            s.passing_door();
        } else {
            // Low, inside -> outside (climb DOWN)
            s.walk_nodir(ctx.point_mid, OrderType::ClimbingLadderDown);
            let climb_end = if ctx.is_soldier_attentive {
                OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
            } else {
                OrderType::TransitionClimbingLadderDownWaitingUpright
            };
            s.walk_nodir(ctx.point_mid, climb_end);
            s.passing_door();
            if ctx.is_pc && ctx.sector_out_forces_crouch {
                s.transition(OrderType::TransitionCrouchingDown);
                s.walk(ctx.point_out, OrderType::WalkingCrouched);
            } else {
                s.walk(ctx.point_out, OrderType::WalkingUpright);
            }
            s.passing_door();
        }
    }
}

// ─── Wall door translation ──────────────────────────────────────────

/// Walk-step tolerance applied to the walk-to-mid step of the
/// high/non-direct wall pass (climb-up).
const TELEPORT_WALL: f32 = 60.0;

fn translate_wall(ctx: &DoorPassContext, s: &mut DoorOrders<'_>) {
    if ctx.is_high {
        if ctx.direct {
            // High, outside -> inside (climb DOWN the wall).  The first
            // walk-to-mid tolerance comes from animation distances
            // (different for crenel vs non-crenel); precomputed in
            // `build_door_pass` and threaded via the two
            // `tol_wall_high_direct_*` ctx fields.
            if !ctx.is_crenel {
                s.walk_rev_tol(
                    ctx.point_mid,
                    OrderType::WalkingUpright,
                    ctx.tol_wall_high_direct_noncrenel,
                );
                s.transition_rev(OrderType::Turning);
                if ctx.is_pc {
                    s.transition_rev(OrderType::TransitionCrouchingDown);
                }
                s.walk_rev_nodir(
                    ctx.point_mid,
                    OrderType::TransitionWaitingCrouchedClimbingWallDown,
                );
            } else {
                // Crenel variant
                s.walk_tol(
                    ctx.point_mid,
                    OrderType::WalkingUpright,
                    ctx.tol_wall_high_direct_crenel,
                );
                s.walk_nodir(
                    ctx.point_mid,
                    OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                );
            }
            s.passing_door();
            s.walk_rev_nodir(ctx.point_in, OrderType::ClimbingWallDown);
            s.passing_door();
        } else {
            // High, inside -> outside (climb UP the wall).
            // `TELEPORT_WALL` (60.0) is set as tolerance on the first
            // walk-to-mid step so the climb-up ends before the waypoint
            // is exactly reached (the animation itself carries the
            // actor past the point).
            s.walk_nodir_tol(ctx.point_mid, OrderType::ClimbingWallUp, TELEPORT_WALL);
            if ctx.is_crenel {
                s.walk_nodir(
                    ctx.point_out,
                    OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
                );
            } else {
                s.walk_nodir(
                    ctx.point_mid,
                    OrderType::TransitionClimbingWallUpWaitingCrouched,
                );
            }
            s.passing_door();
            if ctx.is_pc && !ctx.sector_out_forces_crouch {
                s.transition(OrderType::TransitionCrouchingUp);
            }
            let exit_action = if !ctx.is_pc || !ctx.sector_out_forces_crouch {
                OrderType::WalkingUpright
            } else {
                OrderType::WalkingCrouched
            };
            s.walk(ctx.point_out, exit_action);
            s.passing_door();
        }
    } else {
        if ctx.direct {
            // Low, outside -> inside (climb UP).  Tolerance is the
            // `TransitionWaitingUprightClimbingWallUp` animation
            // distance, precomputed in `build_door_pass` and threaded
            // as `ctx.tol_wall_low_direct`.
            s.walk_tol(
                ctx.point_mid,
                OrderType::WalkingUpright,
                ctx.tol_wall_low_direct,
            );
            s.walk_nodir(
                ctx.point_mid,
                OrderType::TransitionWaitingUprightClimbingWallUp,
            );
            s.passing_door();
            s.walk_nodir(ctx.point_in, OrderType::ClimbingWallUp);
            s.passing_door();
        } else {
            // Low, inside -> outside (climb DOWN)
            s.walk_nodir(ctx.point_mid, OrderType::ClimbingWallDown);
            s.walk_nodir(
                ctx.point_mid,
                OrderType::TransitionClimbingWallDownWaitingUpright,
            );
            s.passing_door();
            if ctx.is_pc && ctx.sector_out_forces_crouch {
                s.transition(OrderType::TransitionCrouchingDown);
                s.walk(ctx.point_out, OrderType::WalkingCrouched);
            } else {
                s.walk(ctx.point_out, OrderType::WalkingUpright);
            }
            s.passing_door();
        }
    }
}

// ─── Stairs door translation ────────────────────────────────────────

fn translate_stairs(ctx: &DoorPassContext, s: &mut DoorOrders<'_>) {
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
        s.push(ctx.point_mid, anim_outside, reverse, true, 0.0);
        s.passing_door();
        s.push(ctx.point_in, anim_inside, reverse, true, 0.0);
        s.passing_door();
    } else {
        // Inside -> outside
        s.push(ctx.point_mid, anim_inside, reverse, true, 0.0);
        s.passing_door();
        s.push(ctx.point_out, anim_outside, reverse, true, 0.0);
        s.passing_door();
    }
}

// ─── Translate default/gate/trap/reinforcement doors ────────────────

fn translate_default(ctx: &DoorPassContext, s: &mut DoorOrders<'_>) {
    let reverse = ctx.is_carrying_on_shoulders;
    let action = ctx.action;

    if !ctx.direct {
        // Inside -> outside
        s.push(ctx.point_mid, action, reverse, true, 0.0);
        s.passing_door();

        // Forced-crouch on exit sector
        if ctx.is_pc && ctx.sector_out_forces_crouch {
            s.transition(OrderType::TransitionCrouchingDown);
            s.walk(ctx.point_out, OrderType::WalkingCrouched);
        } else {
            s.push(ctx.point_out, action, reverse, true, 0.0);
        }
        s.passing_door();
    } else {
        // Outside -> inside
        s.push(ctx.point_mid, action, reverse, true, 0.0);
        s.passing_door();

        // Forced-crouch on entry sector
        if ctx.is_pc && ctx.sector_in_forces_crouch {
            s.transition(OrderType::TransitionCrouchingDown);
            s.walk(ctx.point_in, OrderType::WalkingCrouched);
        } else {
            s.push(ctx.point_in, action, reverse, true, 0.0);
        }
        s.passing_door();
    }
}

/// Return value from [`EngineInner::build_door_pass`].
///
/// Pairs the traversal identity with a post-translation action-recursive
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
    /// Whether this door type's translation marks direct door passage.
    ///
    /// The original game assigns that latch only while translating building-door passage,
    /// The original game's ladder, wall, and stairs door translations. The
    /// `DOOR_REINFORCEMENT / DOOR_DEFAULT /
    /// DOOR_GATE / DOOR_TRAP translation arm
    /// builds its order chain inline and
    /// never touches it, so the latch keeps whatever the actor's previous
    /// building or lift pass left behind.
    sets_passing_door_directly: bool,
}

impl EngineInner {
    pub(super) fn instruct_pass_door(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        entity_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let movement = self
            .orders
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
                    element
                        .legacy_v48
                        .as_ref()
                        .is_some_and(|legacy| !legacy.order_state.is_empty()),
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
            restored_translated_from_v48,
        ) = movement.unwrap_or_else(|| {
            panic!(
                "PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} is not movement data"
            )
        });
        let door_index = gate_id.unwrap_or_else(|| {
            panic!("PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} has no gate")
        });
        let door = self
            .script_domains.interactables.doors
            .get(usize::from(door_index))
            .unwrap_or_else(|| {
                panic!(
                    "PassDoor sequence element {seq_id:?}/{elem_idx} for {entity_id:?} references missing door {door_index}"
                )
            });

        let (actor_sector, auth_info) = self
            .world.entities
            .get(entity_id)
            .map(|entity| (entity.element_data().sector(), entity.actor_auth_info()))
            .unwrap_or_else(|| {
                panic!(
                    "PassDoor sequence element {seq_id:?}/{elem_idx} references missing owner {entity_id:?}"
                )
            });

        // Door traversal disables anti-collision before the
        // direction/authorization switch. Denied and otherwise-impossible
        // attempts therefore leave it disabled until movement teardown.
        self.world
            .entities
            .get_mut(entity_id)
            .expect("PassDoor owner disappeared between canonical lookups")
            .position_iface_mut()
            .set_anti_collision_on(false);

        let actor_sector = actor_sector.unwrap_or_else(|| {
            panic!(
                "PassDoor owner {entity_id:?} has no sector for door {door_index} direction resolution at {seq_id:?}/{elem_idx}"
            )
        });
        // Actor translation resolves the pass direction with a
        // single current-sector versus entrance-sector branch in every door
        // arm; the
        // exit-sector validity check in the other branch is a
        // debug-only check. An actor standing in a third sector — e.g. a
        // building-instance or roof sector that is neither side of the gate —
        // therefore passes the door *directly* in the shipped build.
        let direct = u16::from(actor_sector) != u16::from(door.sector_in);
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
            self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
            return;
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
            self.element_impossible(sim, assets, active_scripts, seq_id, elem_idx);
            return;
        }

        let door_type = door.door_type;
        self.get_entity_mut(entity_id)
            .expect("door instruction owner disappeared")
            .position_iface_mut()
            .set_door(door_index, direct);
        let mut built = self.build_door_pass(
            seq_id,
            elem_idx,
            entity_id,
            door_index,
            direct,
            flags,
            authored_action,
            posture_after_transition,
            action_state_after_transition,
        );
        // A translated PassDoor writes its traversal direction onto the
        // movement element before the actor crosses the gate. Its saved
        // orders prove that translation occurred, so retain that original-game truth
        // value for `Position(actor)` and committed route-source queries even
        // when the actor has already crossed. Queued, untranslated v48
        // PassDoor elements have no orders and their dormant direction word
        // is not authoritative; Original derives those from the live sector.
        if restored_translated_from_v48 {
            built.pass.position_direct = saved_direction != 0;
        }
        // The original PassDoor translation only rewrites the movement
        // element's action for building / default / gate / trap doors and
        // for stairs lifts, and only on the element itself.  Ladder and
        // wall lift passes leave the element's authored action untouched
        // (their step actions are explicit), and nothing propagates to
        // following elements — only the PC forced-crouch override below
        // walks the chain recursively.
        let rewrite_element_action = match door_type {
            DoorType::LiftHigh | DoorType::LiftLow | DoorType::LiftHighCrenel => {
                matches!(lift_type, Some(LiftType::Stairs) | Some(LiftType::Normal))
            }
            _ => true,
        };
        if rewrite_element_action
            && let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
        {
            elem.set_action(built.root_action);
        }
        if let Some(override_action) = built.post_chain_action_recursive {
            self.orders
                .sequence_manager
                .set_action_recursive(seq_id, elem_idx, override_action);
        }
        let actor = self
            .world
            .entities
            .expect_actor_data_mut(entity_id, format_args!("door instruction owner"));
        if built.sets_passing_door_directly {
            actor.passing_door_directly = built.pass.position_direct;
        }
        actor.active_door_pass = Some(built.pass);
        tracing::debug!(
            entity = ?entity_id,
            door = %door_index,
            ?direct,
            "PassDoor: installed door order chain"
        );
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

impl EngineInner {
    /// Append the complete order chain and return its traversal identity.
    ///
    /// Dispatches to the appropriate translate function based on door type
    /// and lift type.
    fn build_door_pass(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
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
            let door = self
                .script_domains
                .interactables
                .doors
                .get(usize::from(door_index))
                .unwrap_or_else(|| {
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
            .world
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

        // Choose the base movement animation. For humans, the original game
        // uses the posture/action state
        // stamped onto the movement element by actor instruction handling, not the live
        // actor state. It starts from the authored action; FAST remains an
        // independent path flag and does not imply that an AI-authored
        // `RunningUpright` PassDoor should walk.
        // Door-pass uses the `WalkingWith*` / `RunningWith*` variants so
        // the stairs translator routes them through the sword/shield
        // branch instead of the plain walk/run branch.
        let is_fast = flags.contains(crate::sequence::MoveFlags::FAST);
        let mut action = if posture_after_transition == Posture::Crouched {
            // Base movement animation selection uses the
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
            // Human movement-animation selection derives sword speed from
            // the authored movement action, not an implied fast-movement flag. Door passing
            // elements deliberately carry empty flags, so consulting FAST
            // here downgraded an authored run at each door boundary.
            match authored_action {
                OrderType::WalkingWithSword | OrderType::RunningWithSword => authored_action,
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    OrderType::WalkingWithSword
                }
                OrderType::RunningUpright => OrderType::RunningWithSword,
                other => panic!(
                    "movement animation selection received unsupported sword PassDoor action {other:?} for {entity_id:?}"
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
                &self.world.fast_grid,
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
        // Wall-door translation wraps both high/direct
        // tolerances in `abs()`; the
        // climb-down transition distances are negative, so without the
        // absolute value the walk-to-mid order can never satisfy
        // the goal-reached `increment . (goal - pos) <= tolerance` test at
        // the ring Original stops on, and the walk overshoots by a frame.
        let tol_wall_high_direct_noncrenel = (dist(OrderType::TransitionCrouchingDown)
            + dist(OrderType::TransitionWaitingCrouchedClimbingWallDown))
        .abs();
        let tol_wall_high_direct_crenel =
            dist(OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel).abs();
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
        // Only building and lift passage mark direct door passage.
        let sets_passing_door_directly = matches!(
            door_type,
            DoorType::Building
                | DoorType::BuildingTrap
                | DoorType::LiftHigh
                | DoorType::LiftHighCrenel
                | DoorType::LiftLow
        );

        let element = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("door translation element disappeared");
        let mut orders = DoorOrders {
            element,
            next_id: &mut self.orders.next_order_id,
        };
        match door_type {
            DoorType::Building | DoorType::BuildingTrap => translate_building(&ctx, &mut orders),
            DoorType::LiftHigh | DoorType::LiftHighCrenel | DoorType::LiftLow => match lift_type {
                Some(LiftType::Ladder) => translate_ladder(&ctx, &mut orders),
                Some(LiftType::Wall) => translate_wall(&ctx, &mut orders),
                Some(LiftType::Stairs) | Some(LiftType::Normal) => {
                    translate_stairs(&ctx, &mut orders)
                }
                None => panic!(
                    "PassDoor owner {entity_id:?} door {door_index} is a lift door but sector {sector_in} has no lift type"
                ),
            },
            _ => translate_default(&ctx, &mut orders),
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
                triggers_fired: 0,
            },
            root_action: action,
            post_chain_action_recursive,
            sets_passing_door_directly,
        }
    }
}

// ─── Engine completion methods ─────────────────────────────

impl EngineInner {
    /// Execute the PassDoor callback — change layer/sector and trigger
    /// building/lift callbacks.
    ///
    /// Called by the selected PassingDoor order.
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
            // Door-passing execution enables anti-collision
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
        let (
            target_layer,
            target_sector_num,
            target_sector_index,
            _door_type,
            is_lift_high,
            door_point_out,
        ) = {
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
            let (tl, ts, ti) = if direct {
                (door.layer_in, door.sector_in, door.sector_in_index)
            } else {
                (door.layer_out, door.sector_out, door.sector_out_index)
            };
            let ti = ti.unwrap_or_else(|| {
                panic!(
                    "PassDoor callback for {entity_id:?} references door {door_index} with no exact target-sector identity"
                )
            });
            let is_high = matches!(
                door.door_type,
                DoorType::LiftHigh | DoorType::LiftHighCrenel
            );
            let pout = door.point_out;
            (tl, ts, ti, door.door_type, is_high, pout)
        };

        // Read the entity's current sector before the change.
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
        // Door traversal reads
        // the current sector purely to run the leave callbacks and then assigns the
        // gate's other side unconditionally. It never requires the departure
        // sector to be the door's nominal source, so an actor that entered the
        // pass from a third sector is legal here too.
        tracing::trace!(
            target: "parity_door_pass",
            entity = ?entity_id,
            door = %door_index,
            direct,
            from = u16::from(current_sector),
            to = u16::from(target_sector_num),
            "PassDoor callback sector change"
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
                // Building-exit hook on enemy AI. The body is
                // currently empty, but the call is wired so future
                // building-door passage logic flows naturally.
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
        let carried = self
            .get_entity(entity_id)
            .expect("PassDoor owner disappeared after canonical callback lookup")
            .pc_data()
            .and_then(|pc| pc.carried);
        let target_sector =
            crate::position_interface::SectorHandle::new(u16::from(target_sector_num));
        {
            let entity = self
                .get_entity_mut(entity_id)
                .expect("PassDoor owner disappeared after canonical callback lookup");
            let elem = entity.element_data_mut();
            elem.set_layer(target_layer);
            elem.sprite
                .position_iface
                .set_sector_topology(target_sector, Some(target_sector_index));
            // Door passing consumes the sprite door pointer on the
            // first PassingDoor callback. A later callback only restores
            // anti-collision and must continue to observe a null door.
            elem.sprite.position_iface.clear_door();
        }
        // Human actor layer-and-sector changes immediately give a
        // PC's carried actor the same layer and sector before refreshing
        // paired jump lines. It does not copy position, obstacle, or material.
        if let Some(carried_id) = carried {
            let carried = self.get_entity_mut(carried_id).unwrap_or_else(|| {
                panic!(
                    "PassDoor owner {entity_id:?} references missing carried actor {carried_id:?}"
                )
            });
            let elem = carried.element_data_mut();
            elem.set_layer(target_layer);
            elem.sprite
                .position_iface
                .set_sector_topology(target_sector, Some(target_sector_index));
        }
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
        // Door passing only runs this refresh inside the
        // non-direct arm: the
        // plane-reselection step and the following full position update live
        // in the branch that switches to the outside sector. A direct pass out
        // of a building sector — which the debug build merely asserts against
        // — keeps its existing obstacle, plane and 3D position.
        if left_building && !direct {
            let target_sector =
                target_sector.expect("validated PassDoor target sector lost its public handle");
            let new_obstacle = self.find_projection_area_at(
                assets,
                target_layer,
                target_sector.with_arena_index(target_sector_index),
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
            // Building-entry hook on enemy AI. See the leave-side
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
        sector: crate::position_interface::SectorHandle,
        point: crate::coordinates::MapPoint,
    ) -> Option<crate::sight_obstacle::SightObstacleIndex> {
        self.get_projection_area_index(assets, sector, layer, point)
    }

    /// Apply a projection-area obstacle + its footstep material to an
    /// actor.
    ///
    /// With `Some(obstacle_idx)`: the actor's sprite takes the
    /// obstacle's material and its top-plane coefficients.  With
    /// `None`: clears the obstacle and falls back to the sound-sector
    /// material at the actor's current position — iterate the sound
    /// sectors the fast-find grid holds **for the actor's own layer**
    /// and pick the material of the first one that contains the point,
    /// or the map's default material when none match
    /// from the position query. This implementation uses
    /// [`crate::material_sectors::MaterialSectors::material_at_layer`]
    /// which encapsulates both steps.
    ///
    /// Updates both `ElementData` (obstacle_index, material) and the
    /// actor's `PositionInterface` (obstacle, plane, material).
    pub(super) fn set_obstacle_and_material(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        obstacle_index: Option<crate::sight_obstacle::SightObstacleIndex>,
    ) {
        let (material, plane) = match obstacle_index {
            Some(idx) => {
                let obs = self.sight_obstacles(assets).get(usize::from(idx));
                let material =
                    obs.map(|o| crate::element::GameMaterial::from_u32(o.material as u32));
                let plane = obs.map(|o| {
                    crate::position_interface::PlaneZCoeffs::from_plane_points(&o.top_plane_points)
                });
                (material, plane)
            }
            None => {
                // No obstacle: clear plane, then resolve footstep
                // material from the sound sectors registered on the
                // actor's own layer at its current map position, with
                // the default material as the fallback.
                let probe = self
                    .get_entity(entity_id)
                    .map(|e| (e.position_iface().map_position(), e.element_data().layer()));
                let material = probe.map(|(point, layer)| {
                    assets
                        .environment
                        .material_sectors
                        .material_at_layer(point, layer)
                });
                (material, None)
            }
        };
        if let Some(entity) = self.get_entity_mut(entity_id) {
            if let Some(mat) = material {
                entity.element_data_mut().set_material(mat);
            }
            let pi = entity.position_iface_mut();
            pi.set_obstacle(obstacle_index, plane);
            if let Some(mat) = material {
                pi.set_material(mat);
            }
        }
    }

    /// Apply the patch associated with a door, if any.
    ///
    /// Executes the patch transition and its terrain updates directly.
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

        self.apply_patch(sim, assets, patch_index);

        tracing::debug!(
            door = %door_index,
            patch = patch_idx,
            "apply_door_patch: patch applied"
        );
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
            .unwrap_or_else(|| panic!("PassDoor references missing canonical sector {sector_num}"))
            .force_crouched
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
#[path = "door_pass/tests.rs"]
mod tests;
