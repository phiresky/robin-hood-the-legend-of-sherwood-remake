//! Phase methods of `EngineInner::tick_one_movement_actor`.
//!
//! This file is a child module of `engine::movement` (declared there with a
//! `#[path]` attribute) rather than a sibling registered in `engine/mod.rs`:
//! the phases use dozens of movement-private helpers, and a child module can
//! reach them without widening their visibility.
//!
//! The shell in `movement.rs` sequences these phases in the exact statement
//! order of the former monolithic body. Every phase that could leave the
//! actor slot early reports that through its return value, and the shell
//! returns immediately. `MovementStepCtx` owns the single mutable entity
//! borrow for the whole Execute (re-looking the entity up would bump its
//! slot generation) next to the disjoint engine borrows the phases need.

use super::*;
use crate::coordinates::GroundPoint;
use crate::position_interface::SectorHandle;
use crate::sequence::MoveFlags;

/// `(position, sector, current gameplay point)` of the FinalTol seek target.
type LiveSeekTarget = Option<(MapPoint, Option<SectorHandle>, Option<MapPoint>)>;
/// `(position, ground, sector, unchanged_or_in_tolerance)` of the actor-owned
/// seek target.
type LiveActorSeekTarget = Option<(MapPoint, GroundPoint, Option<SectorHandle>, bool)>;

/// Target and diagnostic snapshots sampled from the entity table before the
/// movement owner's entity borrow is taken.
#[derive(Clone, Copy)]
pub(super) struct MovementStepEntry {
    live_seek_target: LiveSeekTarget,
    live_seek_target_ground: Option<GroundPoint>,
    actor_seek_flags: MoveFlags,
    live_actor_seek_target: LiveActorSeekTarget,
    provenance_frame: u32,
    diagnostic_creation_order: Option<u32>,
    sprite_row_diagnostic: bool,
    selected_command: crate::element::Command,
}

/// Actor traits sampled once at Execute entry.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct MovementActorTraits {
    is_pc: bool,
    is_drunken_soldier: bool,
    human_is_carried: bool,
    is_swordfighting: bool,
}

/// Selected order plus the entry-only facts derived from it.
#[derive(Clone, Copy)]
pub(super) struct MovementStepSelection {
    pub(super) traits: MovementActorTraits,
    pub(super) order: SelectedMovementOrder,
    pub(super) terminal_pc_external_direction_goal: Option<(i16, i16)>,
}

/// Borrowed engine view and inputs for one movement actor's Execute.
///
/// No serde: this is a borrow bundle, not data.
pub(super) struct MovementStepCtx<'a> {
    pub(super) entity: &'a mut crate::element::Entity,
    pub(super) orders: &'a mut super::state::OrderRuntime,
    pub(super) fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    pub(super) repulsive_points: &'a [crate::ai::RepulsivePoint],
    pub(super) frame_counter: u32,
    pub(super) sim: &'a crate::sim_rng::SimulationContext,
    pub(super) assets: &'a LevelAssets,
    pub(super) owner: EntityId,
    pub(super) actor_id: crate::entity_id::ActorId,
    pub(super) entity_id: EntityId,
    pub(super) prepass: &'a MovementPrepass,
    pub(super) prepared: &'a LiveMobileGeometry,
    pub(super) anti_snapshots: &'a mut EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
    pub(super) deferred: &'a mut MovementDeferred,
    pub(super) entry: MovementStepEntry,
    pub(super) traits: MovementActorTraits,
    pub(super) order: SelectedMovementOrder,
    /// Live copy of `order.order_compute_direction`; climb initialization
    /// clears it while `order` keeps the sampled Execute input.
    pub(super) order_compute_direction: bool,
    pub(super) terminal_pc_external_direction_goal: Option<(i16, i16)>,
}

/// Seek-countdown phase results.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct MovementSeekEntry {
    tolerance_arrival: bool,
    soldier_attentive: bool,
    execute_order_initialising: bool,
}

/// Goal vector and combat-facing classification.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct MovementCombatFacing {
    dx: f32,
    dy: f32,
    dist: f32,
    combat_target: Option<MapPoint>,
    is_sword_motion: bool,
    executes_sword_movement: bool,
    executes_shield_movement: bool,
    is_combat: bool,
}

/// Every pre-motion decision for this Execute.
#[derive(Clone, Copy)]
pub(super) struct MovementMotionPlan {
    seek: MovementSeekEntry,
    facing: MovementCombatFacing,
    anim: OrderType,
    speed_factor: f32,
    is_transition_anim: bool,
    apply_speed_factor: bool,
    fast_climb_motion: bool,
    fast_climb_stops_after_first_termination: bool,
    motion_method: MotionMethod,
    motion_order: Option<MotionOrderContext>,
    dest_already_at_pos: bool,
}

impl MovementMotionPlan {
    pub(super) fn is_transition_without_tolerance_arrival(&self) -> bool {
        self.is_transition_anim && !self.seek.tolerance_arrival
    }
}

/// Sprite-motion results (formerly the first/second fast-call accumulators).
#[derive(Clone, Copy)]
pub(super) struct MovementMotionStep {
    motion_state: MotionState,
    frame_dist_raw: f32,
    first_frame_dist_raw: f32,
    first_direction_differs_from_goal: bool,
    fast_motion_outer_pre: MapPoint,
    first_fast_commit: Option<(MapPoint, MapVec, f32, MapPoint)>,
    second_fast_operands: Option<(MapPoint, MapVec)>,
    second_frame_dist_raw: Option<f32>,
}

/// Effective distance and the state-effect decisions derived from it.
#[derive(Clone, Copy)]
pub(super) struct MovementStepEffects {
    direction_differs_from_goal: bool,
    speed: f32,
    split_motion_speeds: Option<(f32, f32)>,
    entity_target_seek: bool,
    state_effect_motion: MotionState,
    deferred_movement_state_start_due: bool,
    transition_distance_first_execute_due: bool,
}

/// Committed TillLastFrame displacement observations.
#[derive(Clone, Copy)]
struct TransitionCommit {
    crossing_start: Option<(MapPoint, u16, bool)>,
    goal_reached: bool,
}

/// Order-list facts re-read after terminal transition cleanup.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct TransitionSeekCleanup {
    is_final_waypoint_after_transition_cleanup: bool,
    movement_is_last_sequence_element: bool,
}

/// Pre-step crossing snapshot and arrival flags for ordinary motion.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(super) struct MovementArrivalState {
    crossing_old_pos: MapPoint,
    entity_layer: u16,
    entity_posture: crate::element::Posture,
    eligible_for_crossing: bool,
    point_seek_post_arrival: bool,
}

/// Values captured before the ordinary position commit for its diagnostic.
#[derive(Clone, Copy)]
struct OrdinaryStepDiagnosticPre {
    pre_position: MapPoint,
    old_position: MapPoint,
    deviated_before: bool,
    blocked_count_before: u16,
    cached_increment: MapVec,
    anti_on: bool,
}

fn seek_tolerance_reached(
    ft: FinalTol,
    live_seek_target: LiveSeekTarget,
    position: MapPoint,
    self_sector: Option<SectorHandle>,
) -> bool {
    if ft.tol <= 0.0 {
        return false;
    }
    let target_sector = live_seek_target.and_then(|(_, sector, _)| sector);
    if target_sector.is_some() && self_sector != target_sector {
        false
    } else {
        let target_center = ft
            .shield_destination
            .or(live_seek_target.map(|(position, _, _)| position))
            .expect("SEEK FinalTol must have shield_destination or a live target position");
        let target = live_seek_target
            .and_then(|(_, _, point)| point)
            .unwrap_or(target_center);
        let (dx_use, dy_use) = (target.x - position.x, target.y - position.y);
        let dy_effective = if ft.directional {
            const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
            dy_use * INVERSE_ASPECT_RATIO
        } else {
            dy_use
        };
        let dist_sq = dx_use * dx_use + dy_effective * dy_effective;
        dist_sq < ft.tol * ft.tol * 1.1025
    }
}

impl EngineInner {
    /// Sample live seek targets, diagnostics and the selected command before
    /// the owner's entity borrow is taken.
    pub(super) fn snapshot_movement_step_entry(
        &self,
        entity_id: EntityId,
        selected: MovementOwnerSelection,
        prepass: &MovementPrepass,
    ) -> MovementStepEntry {
        let ft = prepass.final_tolerance;
        let live_seek_target = ft.target_id.and_then(|target_id| {
            self.world.entities.get(target_id).map(|target| {
                let target_data = target.element_data();
                let target_position = target_data.position_map();
                let use_point = if ft.use_point {
                    target
                        .current_gameplay_point_map()
                        .filter(|point| *point != target_position)
                } else {
                    None
                };
                (target_position, target_data.sector(), use_point)
            })
        });
        let live_seek_target_ground = ft
            .target_id
            .and_then(|target_id| self.world.entities.get(target_id))
            .map(|target| target.ground_position());
        // Seeking owns its target independently of the copied movement
        // element. A terminating
        // transition can therefore have no FinalTol target while the
        // actor still owns the entity interaction. Keep this snapshot
        // separate: genuine point seeks have no actor-owned target.
        let actor_seek_flags = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "selected seeking owner {entity_id:?} lost movement flags for sequence {:?} element {}",
                    selected.seq_id, selected.elem_idx
                )
            });
        let live_actor_seek_target = self.world.entities.get(entity_id).and_then(|entity| {
            let actor = entity.actor_data()?;
            let target = self.world.entities.get(actor.seek_target?)?;
            let target_position = target.element_data().position_map();
            let sampled_target = actor_seek_flags
                .contains(MoveFlags::USE_POINT)
                .then(|| target.current_gameplay_point_map())
                .flatten()
                .filter(|point| *point != target_position)
                .unwrap_or(target_position);
            let delta = sampled_target - entity.element_data().position_map();
            let stretched_y = if actor_seek_flags.contains(MoveFlags::DIRECTIONAL_TOLERANCE) {
                delta.y * 1.743_446_8
            } else {
                delta.y
            };
            let target_unchanged_or_in_tolerance = target_position
                == actor.last_seek_target_position
                || delta.x * delta.x + stretched_y * stretched_y
                    < actor.seek_distance * actor.seek_distance * 1.1025;
            Some((
                target_position,
                target.ground_position(),
                target.element_data().sector(),
                target_unchanged_or_in_tolerance,
            ))
        });
        let provenance_frame = self.control.frame_counter;
        let diagnostic_creation_order =
            crate::sprite::sprite_row_diagnostic_creation_order(provenance_frame, || {
                self.world.original_creation_order(entity_id)
            });
        let sprite_row_diagnostic = diagnostic_creation_order.is_some();
        let selected_command = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .map(|element| element.command)
            .expect("selected movement element disappeared before Execute");
        MovementStepEntry {
            live_seek_target,
            live_seek_target_ground,
            actor_seek_flags,
            live_actor_seek_target,
            provenance_frame,
            diagnostic_creation_order,
            sprite_row_diagnostic,
            selected_command,
        }
    }
}

impl MovementStepCtx<'_> {
    /// Sample actor traits and select the current movement order. `None`
    /// means the owner has no executable movement order this slot.
    pub(super) fn select_movement_order(
        entity: &mut crate::element::Entity,
        manager: &crate::sequence::SequenceManager,
        selected: MovementOwnerSelection,
        actor_id: crate::entity_id::ActorId,
        entity_id: EntityId,
        entry: &MovementStepEntry,
    ) -> Option<MovementStepSelection> {
        let provenance_frame = entry.provenance_frame;
        super::animation::direction_provenance_snapshot(
            entity.position_iface(),
            entity_id,
            provenance_frame,
            "movement_execute_entry",
        );
        let is_pc = entity.is_pc();
        let is_drunken_soldier = entity.is_soldier()
            && entity
                .npc_data()
                .and_then(|npc| npc.ai_brain.base())
                .is_some_and(|base| base.blood_alcohol > 0);
        let human_is_carried = entity
            .human_data()
            .is_some_and(|human| human.carrier.is_some());
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
        let selected_order = EngineInner::prepare_selected_movement_order(
            entity,
            manager,
            selected,
            actor_id,
            entity_id,
            is_swordfighting,
        )?;
        let SelectedMovementOrder {
            order_id,
            is_final_waypoint,
            order_action,
            order_compute_direction,
            order_reverse,
            ..
        } = selected_order;
        let terminal_pc_external_direction_goal = if is_pc
            && is_final_waypoint
            && matches!(
                order_action,
                OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
            && order_compute_direction
            // A new movement order owns the goal unconditionally:
            // The original game initializes motion with
            // increment computation before any terminal cleanup can
            // observe an external orientation.  Only an already-running
            // order can have been reoriented between Execute calls.
            && order_id.is_some_and(|order_id| {
                entity.element_data().sprite.last_processed_order_id == order_id.get()
            }) {
            let pi = entity.position_iface();
            if !pi.is_increment_all_computed() {
                None
            } else {
                let increment = pi.get_increment();
                let mut movement_direction = vector_to_sector_0_to_15(increment.x, increment.y);
                if order_reverse {
                    movement_direction ^= 8;
                }
                let live_direction_goal = i16::from(pi.get_direction_goal());
                (live_direction_goal != movement_direction)
                    .then_some((live_direction_goal, movement_direction))
            }
        } else {
            None
        };
        Some(MovementStepSelection {
            traits: MovementActorTraits {
                is_pc,
                is_drunken_soldier,
                human_is_carried,
                is_swordfighting,
            },
            order: selected_order,
            terminal_pc_external_direction_goal,
        })
    }

    /// FREEZING and PASSING_DOOR tokens return without sprite motion.
    /// Returns `true` when the Execute is complete.
    pub(super) fn execute_non_sprite_movement_action(&mut self) -> bool {
        let SelectedMovementOrder {
            goal,
            is_final_waypoint,
            order_action,
            move_seq_id,
            move_elem_idx,
            legacy_serialized_order_chain,
            ..
        } = self.order;
        let entity_id = self.entity_id;
        let entity = &mut *self.entity;
        let deferred = &mut *self.deferred;
        if order_action == OrderType::Freezing {
            // `MOVE_WAITING` carries a temporary FREEZING order while
            // the pathfinder owns the request.  The original
            // movement action returns IN_PROGRESS without
            // touching the sprite; this token has no destination-backed
            // motion state to initialize or validate.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
            return true;
        }

        if order_action == OrderType::PassingDoor {
            // Actor action execution returns TERMINATED directly after the door
            // callback; no Sprite method runs for this action point.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
            let eid = entity_id;
            if entity
                .actor_data()
                .expect("door-pass action point owner is not an actor")
                .active_door_pass
                .is_none()
            {
                assert!(
                    legacy_serialized_order_chain,
                    "runtime door-pass action point {order_action:?} for {eid:?} lost its active pass"
                );
                // Original saves the complete translated order queue,
                // PositionInterface door pointer/direction, and the
                // actor's direct flag. It has no separate ActiveDoorPass.
                // Execute that authoritative queue directly: the first
                // PassingDoor consumes the saved door and changes sector;
                // a later one sees no value and merely restores
                // anti-collision.
                let door = entity.position_iface().get_door();
                if let Some(door) = door {
                    let direct = entity.position_iface().get_door_direction();
                    deferred.door_triggers.push((eid, door, direct, 0));
                    entity.position_iface_mut().clear_door();
                } else {
                    entity.position_iface_mut().set_anti_collision_on(true);
                }
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                return true;
            }
            let actor = entity
                .actor_data_mut()
                .expect("door-pass action point owner is not an actor");
            let dp = actor.active_door_pass.as_mut().unwrap_or_else(|| {
                panic!("door-pass action point {order_action:?} for {eid:?} has no active pass")
            });
            let trigger_num = dp.triggers_fired;
            dp.triggers_fired += 1;
            deferred
                .door_triggers
                .push((eid, dp.door_index, dp.direct, trigger_num));

            // A TillLastFrame continuation can force the action-point prefix
            // into the concrete queue. In that case generic order advancement must
            // select the already-queued successor; advancing the lazy tail as
            // well would skip ahead of the copied continuation.
            if !is_final_waypoint {
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                return true;
            }

            let advance =
                EngineInner::advance_door_pass(actor, eid, goal, &mut self.orders.next_order_id);
            match advance {
                DoorPassAdvance::Continue {
                    order_id,
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                } => {
                    let mut order =
                        crate::order::Order::new(action, destination.x, destination.y, order_id);
                    order.reverse = reverse;
                    order.compute_direction = compute_direction;
                    order.tolerance = tolerance;
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Paused { transition_order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, transition_order));
                }
                DoorPassAdvance::ActionPoint { order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Done { completed } => {
                    if let Some((door_index, direct)) = completed {
                        deferred
                            .completed_door_passes
                            .push((eid, door_index, direct));
                    }
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                }
                DoorPassAdvance::NoActive => {
                    panic!(
                        "door-pass action point {order_action:?} for {eid:?} lost its active pass"
                    );
                }
            }
            self.orders.messenger.send(crate::messenger::Message::new(
                crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
            ));
            deferred.order_pops.push((move_seq_id, move_elem_idx));
            return true;
        }
        false
    }

    /// Sample pre-motion seek tolerance, age the seek refresh countdown and
    /// apply climb initialization's direction-computation clear.
    pub(super) fn age_movement_seek_refresh(&mut self) -> MovementSeekEntry {
        let SelectedMovementOrder {
            order_id,
            order_action,
            move_seq_id,
            move_elem_idx,
            active_move_flags,
            ..
        } = self.order;
        let entity_id = self.entity_id;
        let ft = self.prepass.final_tolerance;
        let entity = &mut *self.entity;
        let tolerance_arrival = seek_tolerance_reached(
            ft,
            self.entry.live_seek_target,
            entity.element_data().position_map(),
            entity.element_data().sector(),
        );
        // The original game ages this countdown only during seek movement, and
        // dispatches there by the current animation arm rather than by
        // seeking alone. Cross-sector wall/ladder orders therefore
        // retain the flag but freeze this counter while their Execute
        // arms process motion directly. Keep the actual decrement
        // ahead of transition and zero-motion early returns; a successful
        // pre-motion post-seek arrival is the one path that returns before
        // it.
        let perform_seek_calls = perform_seek_calls_per_execute(order_action);
        if ft.target_id.is_some()
            && active_move_flags.contains(MoveFlags::SEEK)
            && perform_seek_calls != 0
            && !(tolerance_arrival && ft.has_post_seek)
        {
            let actor = entity.actor_data_mut().expect("movement owner is actor");
            let wait_before = actor.seek_refresh_wait;
            for _ in 0..perform_seek_calls {
                actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
            }
            // Original performs this aging directly on the overloaded
            // shared wait-timer field. Keep the Rust ordinary-wait copy in
            // sync while seek owns that legacy scalar so every possible
            // exit (post-seek interaction, a following Move, interruption
            // or cancellation) retains the last wrapped value.
            actor.wait_time = actor.seek_refresh_wait;
            tracing::trace!(
                entity = ?entity_id,
                wait_before,
                wait_after = actor.seek_refresh_wait,
                perform_seek_calls,
                tolerance_arrival,
                has_post_seek = ft.has_post_seek,
                "entity-target seeking aged refresh countdown"
            );
        }

        let soldier_attentive = matches!(entity, crate::element::Entity::Soldier(_))
            && entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
        let execute_order_initialising = entity
            .actor_data()
            .expect("movement owner lost actor initialization state")
            .execute_order_initialising;
        if execute_order_initialising && is_authored_climb_action(order_action) {
            // Every climb execution sets the facing to the lift direction
            // and disables direction computation for the selected order during
            // initialization. Without the clear, motion processing
            // immediately replaces that lift-facing goal with the
            // destination vector. This is observable when a save resumes
            // with the new-order flag set on an already-running climb.
            let order = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front_mut())
                .filter(|order| Some(order.order_id) == order_id)
                .unwrap_or_else(|| {
                    panic!(
                        "initializing climb owner {entity_id:?} lost selected order {order_id:?}"
                    )
                });
            order.compute_direction = false;
            self.order_compute_direction = false;
        }
        MovementSeekEntry {
            tolerance_arrival,
            soldier_attentive,
            execute_order_initialising,
        }
    }

    /// Classify combat motion and face the opponent instead of the
    /// movement direction.
    pub(super) fn apply_combat_movement_facing(&mut self) -> MovementCombatFacing {
        let SelectedMovementOrder {
            goal,
            action_state,
            door_pass_anim,
            order_action,
            ..
        } = self.order;
        let entity_id = self.entity_id;
        let prepass = self.prepass;
        let provenance_frame = self.entry.provenance_frame;
        let elem = self.entity.element_data_mut();
        let dx = goal.x - elem.position_map().x;
        let dy = goal.y - elem.position_map().y;
        let dist = (dx * dx + dy * dy).sqrt();
        // Combat movement: face opponent, select directional
        // animation.  `compute_direction=false` (don't auto-face
        // movement direction), face toward opponent, pick
        // forward/backward/strafe animation based on angle between
        // movement vector and facing vector.
        let combat_target = prepass.combat_face_target;
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let executes_sword_movement = executes_sword_movement_action(door_pass_anim, order_action);
        let executes_shield_movement =
            executes_shield_movement_action(door_pass_anim, order_action);
        let is_combat = executes_shield_movement || is_sword_motion;
        if is_combat {
            // Face opponent instead of movement direction.  Use
            // `set_direction_goal` + per-frame `turn()` rather
            // than instantly snapping facing, so the facing
            // rotates one step per frame toward the opponent.
            if let Some(opp_pos) = combat_target {
                let face_origin = if prepass.combat_face_target_is_ground {
                    let position = elem.position();
                    crate::coordinates::MapPoint::new(position.x, position.y)
                } else {
                    elem.position_map()
                };
                let fdx = opp_pos.x - face_origin.x;
                let fdy = opp_pos.y - face_origin.y;
                tracing::trace!(
                    entity = ?entity_id,
                    frame = self.frame_counter,
                    origin_x = face_origin.x,
                    origin_y = face_origin.y,
                    target_x = opp_pos.x,
                    target_y = opp_pos.y,
                    sector = crate::position_interface::vector_to_sector_0_to_15_iso(fdx, fdy),
                    "combat facing target"
                );
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:before",
                );
                elem.set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
                    fdx, fdy,
                ));
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:after",
                );
            }
        }
        // Ordinary movement does not recompute facing from the remaining
        // map-space goal every tick. The original game stamps
        // the goal once from its normalized 3D increment (including the
        // live ground plane), then returns early while that increment
        // remains valid. Motion processing below owns that initialization;
        // anti-collision and path-boundary code explicitly invalidate and
        // rebuild it when the trajectory actually changes.
        MovementCombatFacing {
            dx,
            dy,
            dist,
            combat_target,
            is_sword_motion,
            executes_sword_movement,
            executes_shield_movement,
            is_combat,
        }
    }

    /// Choose the sprite animation from action state and movement angle.
    pub(super) fn select_movement_animation(
        &self,
        seek: MovementSeekEntry,
        facing_ctx: MovementCombatFacing,
    ) -> OrderType {
        let soldier_attentive = seek.soldier_attentive;
        let SelectedMovementOrder {
            goal,
            action_state,
            door_pass_anim,
            order_action,
            ..
        } = self.order;
        let MovementCombatFacing {
            dx,
            dy,
            combat_target,
            is_sword_motion,
            executes_shield_movement,
            is_combat,
            ..
        } = facing_ctx;
        let entity_id = self.entity_id;
        let prepass = self.prepass;
        let elem = self.entity.element_data();
        if let Some(dp_anim) = door_pass_sprite_animation_override(order_action, door_pass_anim)
            .filter(|anim| !is_sword_movement_nonanimation(*anim))
        {
            // PassDoor supplies the current translated movement step, but
            // Soldier execution still dispatches that logical action
            // through its attentive-animation override. In particular,
            // an attentive WalkingUpright door step plays
            // WalkingAlerted and therefore uses its distinct frame
            // distances.
            super::animation::soldier_movement_animation(dp_anim, soldier_attentive, action_state)
        } else if is_combat {
            if is_sword_motion && combat_target.is_none() {
                // Plain WALKING_SWORD when a non-soldier is forced
                // through sword movement without an active
                // opponent.  The `WalkingWithSword` /
                // `RunningWithSword` values are non-animations and
                // must never be sent directly to the per-frame
                // motion update.
                if elem.sprite.has_animation(OrderType::WalkingSword) {
                    OrderType::WalkingSword
                } else {
                    order_action
                }
            } else {
                // Compute angle between movement direction and
                // facing direction, normalised to [0, 2π).
                // UNIT = π/4 (45°).  8-sector mapping:
                //   [0, π/4) or [7π/4, 2π) → forward
                //   [π/4, 3π/4)             → strafe right
                //   [3π/4, 5π/4)            → backward
                //   [5π/4, 7π/4)            → strafe left
                // The facing vector is the one opponent-facing measures
                // against, so keep it as a vector: reducing it to an angle
                // first would lose the degenerate cases the Original
                // resolves through its determinant test.
                let facing = if let Some(opp_pos) = combat_target {
                    let face_origin = if prepass.combat_face_target_is_ground {
                        let position = elem.position();
                        crate::coordinates::MapPoint::new(position.x, position.y)
                    } else {
                        elem.position_map()
                    };
                    let fdx = opp_pos.x - face_origin.x;
                    let fdy = opp_pos.y - face_origin.y;
                    // Preserve opponent-facing's literal vector, including
                    // the zero vector produced by co-located fighters.
                    // The vector-angle calculation resolves dot == det == 0 to PI,
                    // selecting the backwards-sword animation. Replacing
                    // it with the current heading selects a strafe row.
                    (fdx, fdy)
                } else if executes_shield_movement {
                    // With neither danger point nor protected ally,
                    // Danger-facing retains its default zero vector.
                    (0.0, 0.0)
                } else {
                    let heading = (elem.direction() as f32) * std::f32::consts::PI / 8.0;
                    (heading.cos(), heading.sin())
                };
                let angle = combat_movement_angle((dx, dy), facing);
                // MovingSword and MovingFastSword both use the
                // directional walking/strafing sword animations — the
                // `fast` flag is ignored when selecting the animation.
                // Running in combat is implemented by playing the walking
                // animation under `MotionMethod::Fast`.
                let directional_anim = combat_directional_animation(
                    if executes_shield_movement {
                        crate::element::ActionState::MovingShield
                    } else {
                        action_state
                    },
                    angle,
                );
                tracing::trace!(
                    target: "parity_face_opponent",
                    ?entity_id,
                    goal_x = goal.x,
                    goal_y = goal.y,
                    here_x = elem.position_map().x,
                    here_y = elem.position_map().y,
                    ?combat_target,
                    ground_origin = prepass.combat_face_target_is_ground,
                    facing_x = facing.0,
                    facing_y = facing.1,
                    angle,
                    ?directional_anim,
                    "opponent-facing combat row selection",
                );
                if elem.sprite.has_animation(directional_anim) {
                    directional_anim
                } else {
                    order_action
                }
            }
        } else {
            // Animation comes from the current order's type —
            // dispatch is on `order.action`.  Order types get
            // rewritten by speed or posture changes,
            // so reading the order directly is
            // how a mid-movement speed change propagates to the
            // sprite.  Falls back to an action_state-derived base
            // only when the order type isn't a movement animation
            // (shouldn't happen for a Move element but is
            // defensive).
            let base = literal_lift_sprite_action(order_action).unwrap_or(match order_action {
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
                | OrderType::ClimbingLadderUpAlerted
                | OrderType::ClimbingLadderDownAlerted
                | OrderType::ClimbingLadderUpFast
                | OrderType::ClimbingLadderDownFast
                | OrderType::WalkingWithCorpse
                | OrderType::WalkingCarryingOnShoulders => order_action,
                _ => match action_state {
                    crate::element::ActionState::MovingFast => OrderType::RunningUpright,
                    _ => OrderType::WalkingUpright,
                },
            });
            // Movement-animation selection translates the movement
            // element's primary distance-producing action while it is
            // instructed. Path postprocessing runs afterwards and may insert
            // explicit start/end transition orders; Execute dispatches
            // those transition actions literally even when the actor is
            // standing in a live lift sector. Applying the lift map to a
            // transition here would, for example, turn a walk-to-run
            // transition on stairs back into WalkingStairs.
            //
            // For ordinary distance motion, upright posture takes the
            // upwards mapping unconditionally; on-ladder / on-wall
            // posture chooses upwards vs downwards by dot-producting the
            // ladder vector (`pt_low - pt_high`) with the movement
            // vector. Snapshotted in `lift_translation` so we don't have
            // to re-borrow the grid or door table mid-loop.
            let base =
                if !order_uses_distance_motion(order_action) || is_authored_climb_action(base) {
                    // Movement-animation selection rewrites the movement
                    // element once when it is instructed. Every path order
                    // retains that authored climb direction, even if a later
                    // waypoint briefly bends the other way.
                    base
                } else {
                    match prepass.lift_translation {
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
            // Soldier movement receives the action after
            // Movement-animation selection has translated it for the lift,
            // then substitutes the attentive sprite animation. The order
            // matters for stairs: translating WalkingStairsAlerted again
            // would collapse it back to ordinary WalkingStairs.
            super::animation::soldier_movement_animation(base, soldier_attentive, action_state)
        }
    }

    /// Resolve speed factor and motion method, publish initialization
    /// facing/posture, and build the sprite motion order.
    pub(super) fn plan_movement_motion(
        &mut self,
        seek: MovementSeekEntry,
        facing: MovementCombatFacing,
        anim: OrderType,
    ) -> MovementMotionPlan {
        let SelectedMovementOrder {
            goal,
            action_state,
            order_id,
            door_pass_anim,
            order_action,
            active_move_flags,
            order_tolerance,
            order_reverse,
            order_antagonist,
            next_destination_same_action,
            ..
        } = self.order;
        let MovementSeekEntry {
            execute_order_initialising,
            ..
        } = seek;
        let MovementCombatFacing { dx, dy, .. } = facing;
        let entity_id = self.entity_id;
        let prepass = self.prepass;
        let provenance_frame = self.entry.provenance_frame;
        let live_seek_target = self.entry.live_seek_target;
        let order_compute_direction = self.order_compute_direction;
        let elem = self.entity.element_data_mut();
        let mut speed_factor = prepass.speed_factor;
        // Advance sprite animation and get per-frame distance.
        // Motion processing adds direction to the converted animation row,
        // increments the frame, then reads that row's frame distance
        // only when `frame_count == 0` (the first tick of
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
            let ft = prepass.final_tolerance;
            if ft.tol > 0.0
                && ft.target_is_actor
                && matches!(action_state, crate::element::ActionState::MovingShield)
            {
                let (sdx, sdy) = ft
                    .shield_destination
                    .or(live_seek_target.map(|(position, _, _)| position))
                    .map(|p| (p.x - elem.position_map().x, p.y - elem.position_map().y))
                    .unwrap_or((dx, dy));
                let dist_sq = sdx * sdx + sdy * sdy;
                speed_factor = if dist_sq < 25.0 {
                    1.0
                } else if dist_sq < 100.0 {
                    1.5
                } else {
                    2.0
                };
            }
        }
        let speed_factor = speed_factor;
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
        // Transition execution has two distinct original-game paths.
        // Ordinary transitions process motion directly and retain its
        // default factor of 1. Seek transitions use seeking, which
        // forwards the movement element's speed factor to motion processing.
        let apply_speed_factor = !is_transition_anim || active_move_flags.contains(MoveFlags::SEEK);
        // Human movement selects FAST solely from the
        // current logical movement token. The actor can still be in
        // MOVING_FAST_SWORD when a newly selected WALKING_WITH_SWORD
        // order starts; carrying that old state into the method choice
        // would execute the walking order twice before its START side
        // effect changes the state to MOVING_SWORD.
        let fast_sword_motion = order_action == OrderType::RunningWithSword
            || door_pass_anim == Some(OrderType::RunningWithSword);
        // Fast stairs/ladder/wall actions are non-animation dispatch
        // tokens: the Original executes the ordinary sprite motion
        // twice. Lift
        // translation above may therefore turn an already-authored fast
        // token into its ordinary sprite action; retain the dispatch
        // semantics from the sequence order itself.
        let fast_climb_motion = is_fast_climb_action(order_action) || is_fast_climb_action(anim);
        let fast_climb_stops_after_first_termination =
            fast_climb_stops_after_first_termination(order_action)
                || fast_climb_stops_after_first_termination(anim);
        let motion_method = if is_transition_anim {
            MotionMethod::TillLastFrame
        } else if fast_sword_motion {
            MotionMethod::Fast
        } else {
            MotionMethod::Walk
        };
        if let Some(LiftAnimContext::OnClimb {
            lift_type,
            lift_direction,
            ..
        }) = prepass.lift_translation
            && initialising_climb_uses_lift_direction(anim, lift_type, execute_order_initialising)
        {
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:before",
            );
            elem.set_direction_goal(lift_direction);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:after",
            );
        }
        if let Some(posture) = door_pass_eager_posture(
            anim,
            door_pass_anim.is_some(),
            execute_order_initialising,
            prepass.decorative_building_trap_at_destination,
        ) {
            elem.publish_order_posture(posture);
        }
        if execute_order_initialising && let Some(climb_dir) = prepass.door_pass_climb_direction {
            let dir = if matches!(
                (anim, elem.posture()),
                (
                    OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                    crate::element::Posture::Flying
                )
            ) {
                (climb_dir + 8) & 15
            } else {
                climb_dir
            };
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:before",
            );
            elem.set_direction_goal(dir);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:after",
            );
        }

        let motion_order = order_id.map(|order_id| MotionOrderContext {
            order_id,
            destination: goal,
            reverse: order_reverse,
            tolerance: order_tolerance,
            directional_tolerance: active_move_flags.contains(MoveFlags::DIRECTIONAL_TOLERANCE),
            compute_direction: order_compute_direction,
            next_destination_same_action,
            target_element: order_antagonist,
        });

        if let Some(motion_order) = motion_order
            && let Some(mismatch) = elem.sprite.motion_order_state_mismatch(motion_order)
        {
            panic!(
                "movement order state invariant failed for entity {entity_id:?}, order {order_action:?}, id {}: {mismatch:?}",
                motion_order.order_id
            );
        }

        // Fast stairs/ladder/wall Execute is two literal
        // turn-and-motion pairs in the original game. The second pair is
        // skipped when the first
        // motion terminates; folding it into MotionMethod::Fast would
        // over-rotate on that terminal tick and cannot expose the first
        // call's termination barrier.
        // Original short-circuits a newly initialized non-transition
        // motion only when the destination exactly equals the map position.
        // A near-target continuation must still process motion so its
        // ordinary arrival path snaps and retires it in this owner slot.
        let dest_already_at_pos =
            motion_method != MotionMethod::TillLastFrame && elem.position_map() == goal;
        MovementMotionPlan {
            seek,
            facing,
            anim,
            speed_factor,
            is_transition_anim,
            apply_speed_factor,
            fast_climb_motion,
            fast_climb_stops_after_first_termination,
            motion_method,
            motion_order,
            dest_already_at_pos,
        }
    }

    /// Pre-motion turns and the first sprite motion call. Returns the raw
    /// motion state and frame distance.
    pub(super) fn perform_first_movement_motion(
        &mut self,
        plan: &MovementMotionPlan,
    ) -> (MotionState, f32) {
        let SelectedMovementOrder {
            goal,
            order_action,
            active_move_flags,
            ..
        } = self.order;
        let MovementMotionPlan {
            seek:
                MovementSeekEntry {
                    tolerance_arrival,
                    execute_order_initialising,
                    ..
                },
            facing:
                MovementCombatFacing {
                    combat_target,
                    executes_sword_movement,
                    is_combat,
                    ..
                },
            anim,
            is_transition_anim,
            motion_method,
            motion_order,
            dest_already_at_pos,
            ..
        } = *plan;
        let MovementActorTraits {
            is_pc,
            is_drunken_soldier,
            ..
        } = self.traits;
        let MovementStepEntry {
            provenance_frame,
            diagnostic_creation_order,
            sprite_row_diagnostic,
            ..
        } = self.entry;
        let entity_id = self.entity_id;
        let ft = self.prepass.final_tolerance;
        let sim = self.sim;
        let sprite = &mut self.entity.element_data_mut().sprite;
        let previous_sprite_action = sprite.last_action;
        // Human opponent/danger facing turns before
        // seeking. When the seek continues, it turns a
        // second time immediately before motion processing; when tolerance
        // has already been reached, it returns after only this first
        // turn. A non-soldier without a live opponent returns from
        // opponent-facing before setting a direction or turning.
        if is_combat && combat_target.is_some() && active_move_flags.contains(MoveFlags::SEEK) {
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:before",
            );
            let _ = sprite.position_iface.turn();
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:after",
            );
        }
        // Human's sword-movement Execute arm
        // is the one movement arm that has no
        // `Turn()` of its own: its non-SEEK branch goes straight to
        // motion, and its
        // only pre-motion rotation is the one inside opponent-facing
        // opponent-facing step. That step can return before setting direction
        // and before
        // `Turn` — when a non-soldier is no longer swordfighting, which is
        // exactly the `combat_target.is_none()` case resolved above. A
        // non-interruptible order such as PASS_DOOR keeps that arm selected
        // after removing distant swordfight opponents has emptied the opponent
        // list, so the Original then rotates the actor not at all while its
        // direction goal stays where the door route last left it. Every other
        // branch reaching the block below does turn: ordinary actor execution's
        // movement arms call `Turn()` explicitly in their non-SEEK branch
        // in ordinary movement, seeking calls it in both branches, and danger
        // facing always turns.
        let sword_arm_without_face_turn = executes_sword_movement
            && combat_target.is_none()
            && !active_move_flags.contains(MoveFlags::SEEK);
        // Entity-target seeking returns from its successful
        // pre-motion tolerance branch without processing motion.
        // Besides avoiding displacement, this preserves the prior sprite
        // action and suppresses START-owned side effects such as combat
        // initiative transfer. When post-seek sequence launch succeeds the
        // wrapper returns TERMINATED, however; the surrounding Execute
        // arm must still observe that result so a pending movement-end
        // transition applies its terminal posture/action-state effect
        // before the interaction is instructed.
        let (motion_state, frame_dist_raw) = if tolerance_arrival {
            (
                if ft.has_post_seek {
                    MotionState::Terminated
                } else {
                    MotionState::InProgress
                },
                0.0,
            )
        } else {
            // Entity-target seeking tests its successful tolerance
            // branch before the ordinary turn-and-motion block. Do
            // not advance anti-vibration turning on a terminal tolerance
            // sample whose post-seek sequence is taking over.
            if !sword_arm_without_face_turn
                && should_apply_plain_movement_turn(
                    is_drunken_soldier,
                    active_move_flags,
                    order_action,
                )
            {
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:before",
                );
                let _ = sprite.position_iface.turn();
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:after",
                );
            }
            let diagnostic_pre = sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
            let played_direction = u16::from(sprite.position_iface.get_direction().as_u8());
            let result = sprite.perform_motion(
                sim,
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                played_direction,
                FrameProgression::Default,
                false,
                motion_method,
                dest_already_at_pos,
            );
            if let Some(pre) = diagnostic_pre {
                sprite.emit_sprite_row_diagnostic(
                    "perform_motion",
                    provenance_frame,
                    diagnostic_creation_order.expect("enabled diagnostic has owner"),
                    entity_id.index(),
                    order_action,
                    sprite_motion_order_for_nonanimation(anim),
                    played_direction,
                    FrameProgression::Default,
                    pre,
                    result.0,
                );
            }
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "perform_motion:return",
            );
            // A generated walking- or running-start transition can begin
            // exactly where an anti-collision deviation ended (destination
            // == current map position). The shipped Linux game drops the
            // deviation latch on that zero-distance START tick, so every
            // following in-place `Turn()` rotates immediately in *both*
            // directions — a counter-clockwise first-call rotation from a +2
            // count (Savegame_010 replay-014 frame 1030) rules out the
            // previous count-priming model from task #545. Savegame_032
            // replay-010 additionally proves the running-start case: the
            // visible turn history establishes a -2 count immediately before
            // the aligned start, then its next clockwise shield turn rotates
            // on the first call. Available evidence does not expose the
            // latch update responsible for this save-observable detail; see
            // `clear_deviated_for_aligned_transition_start` for the trace
            // evidence bounding it to exactly this startup initialization.
            // The matching walking-to-waiting exit deliberately preserves
            // the latch (Savegame_023 replay-027, Soldier 136, frame 25195).
            if should_clear_deviated_for_aligned_transition_start(
                is_pc,
                execute_order_initialising,
                is_transition_anim,
                order_action,
                previous_sprite_action,
                sprite.position_iface.is_deviated(),
                sprite.position_iface.map_position(),
                goal,
            ) {
                sprite
                    .position_iface
                    .clear_deviated_for_aligned_transition_start();
            }
            result
        };
        if tolerance_arrival {
            // This seeking branch returns before calling any sprite
            // method. Preserve the wrapper's authoritative Execute result
            // for the actor update just as the non-sprite movement branches
            // above do. Leaving the prior sprite DONE latched causes the
            // successful post-seek sequence termination to be hidden as
            // IN_PROGRESS by the generic entity-seek projection.
            sprite.last_motion_state = Some(motion_state);
        }
        (motion_state, frame_dist_raw)
    }

    /// Fast ladder/wall/stairs second turn-and-motion pair, including the
    /// committed first step it requires.
    pub(super) fn perform_fast_climb_second_motion(
        &mut self,
        plan: &MovementMotionPlan,
        first: (MotionState, f32),
    ) -> MovementMotionStep {
        let MovementMotionPlan {
            seek: MovementSeekEntry {
                tolerance_arrival, ..
            },
            anim,
            speed_factor,
            apply_speed_factor,
            fast_climb_motion,
            fast_climb_stops_after_first_termination,
            motion_order,
            dest_already_at_pos,
            ..
        } = *plan;
        let (mut motion_state, mut frame_dist_raw) = first;
        let goal = self.order.goal;
        let actor_id = self.actor_id;
        let prepass = self.prepass;
        let provenance_frame = self.entry.provenance_frame;
        let sim = self.sim;
        let anti_snapshots = &mut *self.anti_snapshots;
        let sprite = &mut self.entity.element_data_mut().sprite;
        let first_frame_dist_raw = frame_dist_raw;
        let first_direction_differs_from_goal =
            sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
        let fast_motion_outer_pre = sprite.position_iface.map_position();
        let mut first_fast_commit = None;
        let mut second_fast_operands = None;
        // Fast ladder/wall execution contains two separate motion
        // calls, but returns immediately when the first one reaches the
        // order goal. RunningStairs has the same two-call loop without
        // that early return, so its terminal tick still advances the
        // sprite in the second call.
        // Project that first call through the same anti-collision query
        // used by the committed movement below.  Deferring all position
        // work until after both sprite calls otherwise advances the
        // animation counter once too often on a terminal first call; the
        // next climb order can then move one simulation frame early.
        let first_fast_call_terminates = if !tolerance_arrival
            && fast_climb_stops_after_first_termination
            && motion_state != MotionState::Terminated
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                self.repulsive_points,
                self.prepared,
                self.fast_grid,
                goal,
                prepass.goal_target_info,
                first_speed,
            )
        } else {
            false
        };
        let mut second_frame_dist_raw = None;
        if !tolerance_arrival
            && fast_climb_motion
            && motion_state != MotionState::Terminated
            && !first_fast_call_terminates
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            if first_speed != 0.0 {
                first_fast_commit = Some(EngineInner::commit_first_fast_movement_step(
                    sprite,
                    self.order,
                    prepass,
                    actor_id,
                    provenance_frame,
                    first_speed,
                    anti_snapshots,
                    self.repulsive_points,
                    self.prepared,
                    self.fast_grid,
                ));
            }
            let _ = sprite.position_iface.turn();
            let (second_state, second_distance) = sprite.perform_motion(
                sim,
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                u16::from(sprite.position_iface.get_direction().as_u8()),
                FrameProgression::Default,
                false,
                MotionMethod::Walk,
                dest_already_at_pos,
            );
            motion_state = second_state;
            second_frame_dist_raw = Some(second_distance);
            frame_dist_raw += second_distance;
            second_fast_operands = Some((
                sprite.position_iface.map_position(),
                sprite.position_iface.get_increment_map(),
            ));
        }
        MovementMotionStep {
            motion_state,
            frame_dist_raw,
            first_frame_dist_raw,
            first_direction_differs_from_goal,
            fast_motion_outer_pre,
            first_fast_commit,
            second_fast_operands,
            second_frame_dist_raw,
        }
    }

    /// Refresh the mover snapshot and collect post-motion sprite callbacks.
    pub(super) fn publish_movement_motion_callbacks(
        &mut self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
    ) {
        let SelectedMovementOrder {
            door_pass_anim,
            order_action,
            active_move_flags,
            ..
        } = self.order;
        let MovementMotionPlan {
            facing: MovementCombatFacing {
                is_sword_motion, ..
            },
            anim,
            ..
        } = *plan;
        let motion_state = step.motion_state;
        let entity_id = self.entity_id;
        let owner = self.owner;
        let deferred = &mut *self.deferred;
        let sprite = &mut self.entity.element_data_mut().sprite;
        // Motion refreshes the retained target element
        // when a new order is initialized. Anti-collision follows that
        // call in the same actor slot in Original, so the mover snapshot
        // must observe the newly installed order's antagonist now rather
        // than the target retained from the preceding order at the
        // top-of-tick snapshot boundary.
        if let Some(snapshot) = self
            .anti_snapshots
            .get_mut(self.actor_id)
            .and_then(|slot| slot.as_mut())
        {
            snapshot.target_element = sprite.position_iface.target_element();
        }
        deferred.executed_sword_movement = is_sword_motion;
        if self.traits.is_pc {
            deferred
                .executed_pc_movement_actions
                .push((entity_id, order_action));
        }
        if door_pass_anim.is_some()
            && matches!(motion_state, MotionState::Start)
            && matches!(
                anim,
                OrderType::TransitionClimbingLadderUpWaitingCrouched
                    | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            )
        {
            deferred.door_pass_transition_start_effects.push(entity_id);
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
                    | OrderType::TransitionClimbingLadderUpWaitingCrouched
                    | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            )
        {
            deferred.door_pass_transition_done_effects.push(entity_id);
        }
        if active_move_flags.contains(MoveFlags::RIDER_CHARGE) && anim == OrderType::RunningUpright
        {
            let frame_count = sprite.num_frames_for_anim(OrderType::RunningUpright);
            let cur = sprite.current_frame;
            if is_galopp_decision_frame(cur, frame_count) {
                assert_eq!(
                    entity_id, owner,
                    "owner-local rider Execute collected a gallop callback for another actor"
                );
                deferred.galopp_event = true;
            }
        }
    }

    /// Scale the raw frame distance and decide which Execute state effects
    /// this motion result publishes.
    pub(super) fn resolve_movement_step_effects(
        &mut self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
    ) -> MovementStepEffects {
        let SelectedMovementOrder {
            goal,
            action_state,
            order_id,
            order_action,
            move_seq_id,
            move_elem_idx,
            active_move_flags,
            order_tolerance,
            transition_distance_continuation,
            deferred_movement_state_start,
            ..
        } = self.order;
        let MovementMotionPlan {
            seek: MovementSeekEntry {
                tolerance_arrival, ..
            },
            facing:
                MovementCombatFacing {
                    dist,
                    executes_sword_movement,
                    ..
                },
            speed_factor,
            is_transition_anim,
            apply_speed_factor,
            ..
        } = *plan;
        let MovementMotionStep {
            motion_state,
            frame_dist_raw,
            first_frame_dist_raw,
            first_direction_differs_from_goal,
            first_fast_commit,
            second_frame_dist_raw,
            ..
        } = *step;
        let entity_id = self.entity_id;
        let actor_id = self.actor_id;
        let prepass = self.prepass;
        let ft = prepass.final_tolerance;
        let anti_snapshots = &*self.anti_snapshots;
        let deferred = &mut *self.deferred;
        let sprite = &mut self.entity.element_data_mut().sprite;
        // Motion processing applies the sequence speed factor before its
        // turn slowdown and 0.7-unit minimum. The order is observable:
        // a slow patrol member with raw distance 2 and factor ~0.58 is
        // clamped to exactly 0.7 after the 0.6 multiplier, rather than
        // scaling an already-clamped 0.7 back below the minimum.
        //
        // Motion processing initializes a new order's direction goal after
        // the caller's turn above. The original-game slowdown test
        // happens later and reads the now-live direction/goal pair, so it
        // applies even though the pre-initialization Turn was a no-op.
        let direction_differs_from_goal =
            sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
        // Direct transitions run motion until the last frame without a speed
        // factor. Seeking transitions pass a speed factor to their motion step.
        let (speed, split_motion_speeds) = if let Some(second_distance) = second_frame_dist_raw {
            // The fast stairs/ladder/wall arms contain two literal
            // motion steps. Each step applies its own turning
            // slowdown using the direction reached by the immediately
            // preceding Turn(), so a first call that is still rotating
            // must not inherit the second call's newly aligned state.
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            let second_speed = scaled_motion_distance(
                second_distance,
                speed_factor,
                apply_speed_factor,
                direction_differs_from_goal,
            );
            (
                if first_fast_commit.is_some() {
                    second_speed
                } else {
                    first_speed + second_speed
                },
                Some((first_speed, second_speed)),
            )
        } else {
            (
                scaled_motion_distance(
                    frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    direction_differs_from_goal,
                ),
                None,
            )
        };
        // Motion processing applies the distance before returning its motion
        // state. A fresh walking order that reaches its goal on that same
        // invocation returns TERMINATED, not START, so the walking
        // Execute arm does not enter the Moving action state. Our
        // position update is staged below; fold that imminent arrival
        // into the state-effect result now.
        let entity_target_seek =
            active_move_flags.contains(MoveFlags::SEEK) && ft.target_id.is_some();
        // The ordinary (non-TillLastFrame) arrival branch runs only when
        // the sprite actually advanced the actor, and it asks the position
        // interface rather than comparing straight-line distances. A
        // walker that sidesteps a neighbour covers more ground than
        // remains to its goal and still ends the frame short of it.
        let reaches_goal_this_step = !is_transition_anim
            && projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                self.repulsive_points,
                self.prepared,
                self.fast_grid,
                goal,
                prepass.goal_target_info,
                speed,
            );
        let state_effect_motion = movement_execute_visible_motion(
            order_action,
            motion_state,
            reaches_goal_this_step,
            entity_target_seek,
        );
        deferred.post_completion_motion_override = committed_arrival_post_completion_override(
            motion_state,
            state_effect_motion,
            reaches_goal_this_step,
        );
        let deferred_movement_state_start_due = if deferred_movement_state_start {
            let current_order = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front_mut())
                .unwrap_or_else(|| {
                    panic!(
                        "deferred movement-state successor for {entity_id:?} disappeared during execution"
                    )
                });
            assert_eq!(
                Some(current_order.order_id),
                order_id,
                "deferred movement-state successor changed identity during execution"
            );
            assert!(
                take_deferred_movement_state_start(
                    &mut current_order.deferred_movement_state_start
                ),
                "deferred movement-state successor marker was already consumed"
            );
            true
        } else {
            false
        };
        // The initiative handoff belongs to the Human Execute START arm,
        // so it observes entity-target seeking's wrapper result just
        // like posture/action-state changes do. A raw sprite START hidden
        // as in progress by seeking must not transfer initiative.
        if matches!(state_effect_motion, MotionState::Start) && executes_sword_movement {
            deferred.sword_movement_starts.push(entity_id);
        }
        tracing::trace!(
            entity = ?entity_id,
            frame = self.frame_counter,
            ?order_action,
            ?motion_state,
            ?state_effect_motion,
            action_state = ?action_state,
            sprite_frame = sprite.current_frame,
            sprite_counter = sprite.frame_count,
            sprite_num_frames = sprite.num_frames_for_row(sprite.current_row),
            sprite_wait = sprite.wait_time(sprite.current_row, sprite.current_frame),
            frame_distance_raw = frame_dist_raw,
            speed_factor,
            effective_distance = speed,
            remaining_distance = dist,
            reaches_goal_this_step,
            order_tolerance,
            deviated = sprite.position_iface.is_deviated(),
            anti_collision = sprite.position_iface.is_anti_collision_on(),
            goal_x = goal.x,
            goal_y = goal.y,
            increment_x = sprite.position_iface.get_increment_map().x,
            increment_y = sprite.position_iface.get_increment_map().y,
            "movement Execute result"
        );
        // Transition motion can still change from InProgress to
        // Terminated in the TILL_LAST_FRAME arrival handling below.
        // Apply the execution state's side effect after
        // Motion processing returns that final result, before advancement rewrites
        // the diagnostic motion for a successor order.
        let transition_distance_first_execute_due = if transition_distance_continuation {
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "transition-distance continuation for {entity_id:?} disappeared during its first execution"
                    )
                });
            let current_order = element.orders.front_mut().unwrap_or_else(|| {
                panic!(
                    "transition-distance continuation for {entity_id:?} lost its current order during its first execution"
                )
            });
            assert_eq!(
                Some(current_order.order_id),
                order_id,
                "transition-distance continuation changed identity during its first execution"
            );
            take_transition_distance_first_execute(
                &mut current_order.transition_distance_continuation,
            )
        } else {
            false
        };
        let suppress_transition_continuation_start = transition_distance_first_execute_due
            && matches!(state_effect_motion, MotionState::Start);
        if !is_transition_anim
            && !suppress_transition_continuation_start
            // A deferred PC successor deliberately postpones this
            // START-only state effect until after order completion has
            // decided whether the authored walking order survived.  The
            // guarded handoff below owns that one-shot side effect.
            && !deferred_movement_state_start_due
            // Motion processing commits the physical step before returning.
            // A fresh order can therefore reach its goal and return
            // TERMINATED instead of exposing START to Execute. Defer all
            // START-only effects until the committed step has decided
            // whether this exact order survives.
            && !matches!(state_effect_motion, MotionState::Start)
            && let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
        {
            deferred
                .movement_state_effects
                .push((entity_id, posture, action_state));
        }
        if is_transition_anim
            && tolerance_arrival
            && let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
        {
            if ft.launches_post_seek {
                // Post-seek sequence launch runs synchronously inside
                // seeking. Its interaction callbacks must therefore see
                // the pre-transition action state. The surrounding transition
                // Execute switch applies TERMINATED only after that recursive
                // work returns.
                deferred
                    .post_seek_terminal_state_effects
                    .push((entity_id, posture, action_state));
            } else if ft.has_post_seek {
                deferred.sequence_seek_terminal_state_effects.push((
                    entity_id,
                    posture,
                    action_state,
                ));
            } else {
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, action_state));
            }
        }
        MovementStepEffects {
            direction_differs_from_goal,
            speed,
            split_motion_speeds,
            entity_target_seek,
            state_effect_motion,
            deferred_movement_state_start_due,
            transition_distance_first_execute_due,
        }
    }

    /// Debug trace of door-pass climbing movement.
    pub(super) fn trace_door_pass_movement_state(
        &self,
        plan: &MovementMotionPlan,
        effects: &MovementStepEffects,
    ) {
        let SelectedMovementOrder {
            action_state,
            door_pass_anim,
            ..
        } = self.order;
        let anim = plan.anim;
        let dist = plan.facing.dist;
        let speed = effects.speed;
        let entity_id = self.entity_id;
        let elem = self.entity.element_data();
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
                entity = ?entity_id,
                ?anim,
                posture = ?elem.posture(),
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
    // to trigger begin_swordfight). The original game routes every nonzero
    // transition distance through collision-aware position updates, so
    // transition displacement must also participate in elevation,
    // patch, and sound boundary crossing.
    //
    // The original game seeds position state at the start of every new
    // sprite motion order and advances transition animations by their
    // frame distance, so this branch uses the
    // same precomputed map increment instead of a separate
    // dx/dy step.
    // Entity-target seeking checks its live tolerance before it
    // dispatches the current sprite order.  An already-in-range seek
    // therefore bypasses transition execution and enters the shared
    // post-seek/frozen arrival tail below.
    /// TillLastFrame Execute tail. The actor slot always ends here.
    pub(super) fn tick_movement_transition(
        &mut self,
        plan: &MovementMotionPlan,
        step: &mut MovementMotionStep,
        effects: &MovementStepEffects,
    ) {
        let commit = self.commit_transition_displacement(plan, step, effects);
        self.publish_transition_effects(plan, step, effects, commit);
        if matches!(step.motion_state, MotionState::Terminated) {
            if let Some((external_direction, movement_direction)) =
                self.terminal_pc_external_direction_goal
            {
                self.deferred.terminal_pc_direction_goal_restores.push((
                    self.entity_id,
                    external_direction,
                    movement_direction,
                ));
            }
            let mut discarded_lazy_door_followers = false;
            // TillLastFrame can exhaust its animation before its
            // distance target is reached (notably the short
            // Waiting→Walking startup transition). The Original does
            // not discard that remaining distance: it copies the
            // current order at the first following animation change,
            // changes the copy to that next animation, then retires
            // the exhausted transition. This keeps the copied order's
            // old target as a one-tick continuation.
            if !commit.goal_reached {
                discarded_lazy_door_followers =
                    self.insert_transition_distance_continuation(effects.entity_target_seek);
            }
            let Some(cleanup) = self.hand_off_terminated_transition_seek(plan) else {
                return;
            };
            if self.hand_off_actor_owned_post_seek(cleanup) {
                return;
            }
            self.retire_terminated_transition(discarded_lazy_door_followers);
        }
    }

    /// Commit TillLastFrame displacement and its goal-arrival snap.
    fn commit_transition_displacement(
        &mut self,
        plan: &MovementMotionPlan,
        step: &mut MovementMotionStep,
        effects: &MovementStepEffects,
    ) -> TransitionCommit {
        let SelectedMovementOrder {
            goal,
            order_action,
            order_tolerance,
            order_reverse,
            next_destination_same_action,
            ..
        } = self.order;
        let MovementMotionPlan {
            facing: MovementCombatFacing { dist, .. },
            anim,
            is_transition_anim,
            ..
        } = *plan;
        let MovementStepEffects {
            speed,
            split_motion_speeds,
            ..
        } = *effects;
        let entity_id = self.entity_id;
        let actor_id = self.actor_id;
        let prepass = self.prepass;
        let provenance_frame = self.entry.provenance_frame;
        let human_is_carried = self.traits.human_is_carried;
        let anti_snapshots = &mut *self.anti_snapshots;
        let entity = &mut *self.entity;
        let transition_has_map_target = goal.x != 0.0 || goal.y != 0.0;
        if !transition_has_map_target && !is_in_place_movement_transition(order_action) {
            panic!(
                "movement transition {:?} for entity {:?} has zero map target; refusing to treat (0,0) as an implicit destination",
                order_action, entity_id
            );
        }
        // A movement transition can legitimately target the actor's
        // exact current point (for example the generated
        // Waiting→Walking pose at the end of a combat sequence).
        // Motion processing still advances that animation, but the zero
        // goal vector contributes no map displacement.  In
        // particular, do not feed a stale pre-order increment into
        // anti-collision: increment computation deliberately preserves
        // the stored vector when the new vector is zero.
        let transition_has_distance =
            transition_has_map_target && speed > 0.0 && dist > f32::EPSILON;
        let transition_recomputes_exact_position = motion_recomputes_exact_position(
            is_transition_anim,
            transition_has_map_target,
            speed,
            dist,
        );
        let transition_crossing_start = transition_has_distance.then(|| {
            let old_pos = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let eligible = actor_line_crossing_eligible(
                entity.element_data().posture(),
                human_is_carried,
                self.fast_grid.level.map_bbox.contains_point(old_pos),
            );
            (old_pos, layer, eligible)
        });
        if transition_has_distance {
            // Match the map increment: motion processing seeded this
            // normalized vector when the order began and reuses it
            // unchanged until anti-collision explicitly rebuilds it.
            let increment = entity.position_iface().get_increment_map();
            let nx = increment.x;
            let ny = increment.y;
            let anti_on = entity.position_iface().is_anti_collision_on();
            // The fast stairs/ladder/wall tokens process motion
            // twice. With anti-collision disabled, Original stores the
            // first position update before applying the second one.
            // Combining both distances and rounding only the final
            // sum moves large map coordinates by an ULP and can
            // amplify into a visible elevation error on steep planes.
            let split_motion_target =
                split_motion_speeds
                    .filter(|_| !anti_on)
                    .map(|(first_speed, second_speed)| {
                        let mut target = entity.element_data().position_map();
                        target.x += nx * first_speed;
                        target.y += ny * first_speed;
                        target.x += nx * second_speed;
                        target.y += ny * second_speed;
                        target
                    });
            let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
            let (move_box, half_diagonal) = {
                let pi = entity.position_iface();
                (*pi.get_move_box(), pi.get_half_diagonal())
            };
            let (dx_step, dy_step, deviated, recovered_from_deviation) = if let Some(mover_snap) =
                anti_snapshots
                    .get(actor_id)
                    .and_then(|slot| slot.as_ref())
                    .filter(|snapshot| snapshot.active)
            {
                let pi = entity.position_iface_mut();
                let was_deviated = pi.is_deviated();
                let mut state = super::anti_collision::AntiCollisionState {
                    pi,
                    move_box,
                    half_diagonal,
                    goal_map,
                };
                let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                    provenance_frame,
                    mover_snap,
                    anti_snapshots,
                    self.repulsive_points,
                    self.prepared,
                    self.fast_grid,
                    &mut state,
                    nx,
                    ny,
                    speed,
                    anti_on,
                );
                (
                    dx_step,
                    dy_step,
                    // Only a committed deviation (blocked counter
                    // reset) faces along its step and rebuilds the
                    // increment here; a break-through barge keeps
                    // the facing and cached increment the
                    // anti-collision step left behind.
                    state.pi.is_deviated() && state.pi.blocked_count == 0,
                    was_deviated && !state.pi.is_deviated(),
                )
            } else {
                (nx * speed, ny * speed, false, false)
            };
            let elem = entity.element_data_mut();
            if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
            }
            let position = split_motion_target.unwrap_or_else(|| {
                let mut position = elem.position_map();
                position.x += dx_step;
                position.y += dy_step;
                position
            });
            elem.set_position_map(position);
            if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                elem.sprite.position_iface.reset_increment_computed();
                elem.sprite.position_iface.compute_increment_all(false);
            } else if recovered_from_deviation {
                // Original rebuilds the trajectory even when this
                // animation frame contributes no movement.
                elem.sprite.position_iface.reset_increment_computed();
                elem.sprite.position_iface.compute_increment_all(true);
            }
            elem.update_grid_cell();
        } else if transition_recomputes_exact_position {
            // Motion processing gates its position update on animation
            // distance, not on the length of the normalized map
            // increment. With an exact-position transition target a
            // nonzero sprite-frame distance therefore still reaches
            // collision-aware position updates with a zero increment. That
            // call is observable even though it cannot displace the
            // actor: its empty-candidate recovery drops a preceding
            // deviation latch before recomputing positions. Skipping the
            // call left a stopping soldier in turn-vibration suppression on the
            // following frame (Linux Savegame_036 replay-015, Soldier
            // 144), delaying the visible counter-clockwise turn.
            let recovered_from_deviation = if entity.position_iface().is_anti_collision_on()
                && let Some(mover_snap) = anti_snapshots
                    .get(actor_id)
                    .and_then(|slot| slot.as_ref())
                    .filter(|snapshot| snapshot.active)
            {
                let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                let (move_box, half_diagonal) = {
                    let pi = entity.position_iface();
                    (*pi.get_move_box(), pi.get_half_diagonal())
                };
                let pi = entity.position_iface_mut();
                let was_deviated = pi.is_deviated();
                let mut state = super::anti_collision::AntiCollisionState {
                    pi,
                    move_box,
                    half_diagonal,
                    goal_map,
                };
                let step = apply_prepared_anti_collision_step(
                    provenance_frame,
                    mover_snap,
                    anti_snapshots,
                    self.repulsive_points,
                    self.prepared,
                    self.fast_grid,
                    &mut state,
                    0.0,
                    0.0,
                    speed,
                    true,
                );
                debug_assert_eq!(step, (0.0, 0.0));
                was_deviated && !state.pi.is_deviated()
            } else {
                false
            };
            let position = entity.element_data().position_map();
            let elem = entity.element_data_mut();
            elem.set_position_map(position);
            if recovered_from_deviation {
                elem.sprite.position_iface.reset_increment_computed();
                elem.sprite.position_iface.compute_increment_all(true);
            }
            elem.update_grid_cell();
            // The same nonzero-animation-distance block ends with
            // forecasted-movement updates even though the cached
            // increment is zero at the goal. This clears a preceding
            // running forecast before projectile leading samples it.
            refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
        }
        if transition_has_distance {
            // The original game's shared motion path refreshes target
            // leading after every committed transition displacement,
            // before goal-arrival testing can clear the live increment. A
            // missing refresh here made arrows aim at the target's
            // current point during start/stop transitions.
            refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
        }
        // TILL_LAST_FRAME still performs the ordinary arrival check
        // after every nonzero transition step. Reaching the target
        // zeros both increments and snaps an undeviated zero-tolerance
        // actor, but the transition keeps playing until its animation
        // loops unless the next order uses the same animation.
        let transition_goal_reached = entity
            .position_iface()
            .is_goal_reached(self.fast_grid, prepass.goal_target_info);
        let transition_increment_nonzero = {
            let increment = entity.position_iface().get_increment_map();
            increment.x != 0.0 || increment.y != 0.0
        };
        if transition_goal_reached && speed != 0.0 && transition_increment_nonzero {
            let should_snap = !entity.position_iface().is_deviated() && order_tolerance == 0.0;
            entity.position_iface_mut().zero_all_increments();
            tracing::trace!(
                ?entity_id,
                ?anim,
                ?goal,
                should_snap,
                from = ?entity.element_data().position_map(),
                "transition goal reached"
            );
            if should_snap {
                entity.element_data_mut().set_position_map(goal);
            }
            if next_destination_same_action.is_some() {
                step.motion_state = MotionState::Terminated;
            }
        }
        TransitionCommit {
            crossing_start: transition_crossing_start,
            goal_reached: transition_goal_reached,
        }
    }

    /// Queue transition crossings and the transition's Execute state effects.
    fn publish_transition_effects(
        &mut self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
        effects: &MovementStepEffects,
        commit: TransitionCommit,
    ) {
        let SelectedMovementOrder {
            door_pass_anim,
            order_action,
            legacy_serialized_order_chain,
            ..
        } = self.order;
        let anim = plan.anim;
        let motion_state = step.motion_state;
        let entity_target_seek = effects.entity_target_seek;
        let selected_command = self.entry.selected_command;
        let is_pc = self.traits.is_pc;
        let entity_id = self.entity_id;
        let entity = &mut *self.entity;
        let deferred = &mut *self.deferred;
        // The actor update runs line-crossing detection after execution
        // returns, so the segment endpoint is resolved from the live
        // position at dispatch time. A TillLastFrame step may
        // overshoot and snap back to its goal; the discarded
        // overshoot must not trigger a boundary.
        if let Some((old_pos, layer, eligible)) = commit.crossing_start
            && eligible
        {
            deferred.line_cross_checks.push((entity_id, old_pos, layer));
            deferred
                .non_elevation_cross_checks
                .push((entity_id, old_pos, layer));
        }
        let transition_effect_motion =
            movement_execute_visible_motion(order_action, motion_state, false, entity_target_seek);
        if let Some((posture, next_action_state)) =
            movement_execute_state_effect(order_action, transition_effect_motion)
        {
            // A speed-transition completion establishes the live
            // walking/running state itself. Do not let an older state
            // saved by a preceding door transition overwrite it when
            // the generated continuation order executes next tick.
            if next_action_state.is_moving()
                && let Some(pass) = entity
                    .actor_data_mut()
                    .and_then(|actor| actor.active_door_pass.as_mut())
            {
                pass.saved_action_state = None;
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        let door_transition_state_effect_due = matches!(motion_state, MotionState::Terminated)
            || matches!(motion_state, MotionState::Done)
                && matches!(
                    anim,
                    OrderType::TransitionClimbingLadderDownWaitingUpright
                        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                );
        if pass_door_transition_completion_has_owner(
            selected_command,
            door_pass_anim.is_some()
                || (legacy_serialized_order_chain
                    && selected_command == crate::element::Command::PassDoor),
            order_action,
            is_pc,
        ) && door_transition_state_effect_due
            && matches!(
                anim,
                OrderType::TransitionWaitingUprightClimbingWallUp
                    | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                    | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
                    | OrderType::TransitionClimbingLadderDownWaitingUpright
                    | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                    | OrderType::TransitionClimbingWallUpWaitingCrouched
                    | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDown
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                    | OrderType::TransitionClimbingWallDownWaitingUpright
            )
        {
            deferred
                .door_pass_transition_completion_effects
                .push((entity_id, order_action));
        }
    }

    /// Copy the exhausted transition as a one-tick continuation using the
    /// next distinct animation. Returns whether lazy door followers were
    /// discarded.
    fn insert_transition_distance_continuation(&mut self, entity_target_seek: bool) -> bool {
        let SelectedMovementOrder {
            order_action,
            move_seq_id,
            move_elem_idx,
            ..
        } = self.order;
        let is_pc = self.traits.is_pc;
        let entity = &mut *self.entity;
        let mut discarded_lazy_door_followers = false;
        // Original's movement element already contains the
        // whole PassDoor route. Rust keeps the untranslated
        // tail on ActiveDoorPass, so the next distinct
        // destination animation may live there rather than in
        // `element.orders`.
        let lazy_next_animation = entity
            .actor_data()
            .and_then(|actor| actor.active_door_pass.as_ref())
            .and_then(|pass| {
                pass.steps.iter().find_map(|step| match step {
                    crate::element::DoorPassStep::Walk {
                        destination,
                        action,
                        ..
                    } if *destination != MapPoint::ZERO && *action != order_action => Some(*action),
                    _ => None,
                })
            });
        let concrete_door_prefix = if lazy_next_animation.is_some() {
            entity
                .actor_data_mut()
                .and_then(|actor| actor.active_door_pass.as_mut())
                .map(|pass| {
                    materialize_door_action_point_prefix(pass, &mut self.orders.next_order_id)
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut continuation_door_action = None;
        let mut discard_lazy_door_followers = false;
        if let Some((element, next_order_id)) = self
            .orders
            .element_with_order_ids_mut(move_seq_id, move_elem_idx)
        {
            for order in concrete_door_prefix {
                element.push_order(order);
            }
            let current_action = element
                .orders
                .front()
                .expect("terminated movement transition lost its current order")
                .order_type;
            let next_animation = element
                .orders
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, order)| {
                    order.order_type != current_action
                        && (order.target_x != 0.0 || order.target_y != 0.0)
                })
                .map(|(index, order)| (index, order.order_type));
            let next_animation = next_animation
                .or_else(|| lazy_next_animation.map(|animation| (element.orders.len(), animation)));
            if let Some((insertion, animation)) = next_animation {
                let mut continuation = element.orders.front().unwrap().clone();
                continuation.order_type = animation;
                // Motion through the last frame can exhaust
                // the transition animation before reaching its
                // distance target. Original inserts this
                // changed-animation copy from inside that same
                // motion step. Defer the copy's initial
                // state effect until we know whether the copy
                // actually survives its first Execute.
                continuation.transition_distance_continuation = true;
                continuation.reseed_id(crate::order::alloc_order_id(next_order_id));
                continuation_door_action = Some((animation, continuation.reverse));
                element.insert_order(insertion, continuation);
                if should_defer_pc_movement_state_start(is_pc, entity_target_seek)
                    && let Some(authored_successor) = element.orders.get_mut(insertion + 1)
                {
                    assert_eq!(
                        authored_successor.order_type, animation,
                        "transition-distance continuation must precede the authored order whose animation it continues"
                    );
                    authored_successor.deferred_movement_state_start = true;
                }
            } else {
                element.orders.truncate(1);
                discard_lazy_door_followers = true;
            }
        }
        if discard_lazy_door_followers {
            discard_lazy_door_pass_following_orders(
                entity
                    .actor_data_mut()
                    .and_then(|actor| actor.active_door_pass.as_mut()),
            );
            discarded_lazy_door_followers = true;
        }
        // Original stores the complete translated door route
        // in the movement element, so changing to this copied
        // successor changes the one authoritative current
        // action. Rust keeps the untranslated route tail in a
        // parallel ActiveDoorPass. Keep its animation mirror
        // in lockstep with the concrete continuation order:
        // lift handling and the next Execute slot both consult
        // it before dispatching sprite motion.
        if let Some((animation, reverse)) = continuation_door_action
            && let Some(pass) = entity
                .actor_data_mut()
                .and_then(|actor| actor.active_door_pass.as_mut())
        {
            pass.current_action = animation;
            pass.current_reverse = reverse;
        }
        discarded_lazy_door_followers
    }

    /// Entity-seek handoff after a terminated transition. `None` means the
    /// Execute is complete.
    fn hand_off_terminated_transition_seek(
        &mut self,
        plan: &MovementMotionPlan,
    ) -> Option<TransitionSeekCleanup> {
        let SelectedMovementOrder {
            move_seq_id,
            move_elem_idx,
            ..
        } = self.order;
        let tolerance_arrival = plan.seek.tolerance_arrival;
        let MovementStepEntry {
            live_seek_target,
            live_seek_target_ground,
            ..
        } = self.entry;
        let ft = self.prepass.final_tolerance;
        let entity = &mut *self.entity;
        let deferred = &mut *self.deferred;
        let eid = self.entity_id;
        // Seeking wraps the transition animation too. When
        // the last stop transition terminates, Original checks
        // the live target before retiring the movement: an
        // unchanged same-sector target completes the seek and
        // starts its actor-owned post-seek interaction.
        //
        // The ordinary walking-arrival path below performs this
        // same check, but transition animations return through
        // this earlier branch and must close the handoff here.
        // Motion through the last frame may have deleted every
        // same-animation follower above after looping short of the
        // current destination. The original game's seeking asks
        // the next order only after that synchronous cleanup, so
        // the just-truncated current order is now the final
        // waypoint even when it was not final at Execute entry.
        let is_final_waypoint_after_transition_cleanup = self
            .orders
            .sequence_manager
            .get_element(move_seq_id, move_elem_idx)
            .is_none_or(|element| element.orders.len() <= 1);
        let movement_is_last_sequence_element = self
            .orders
            .sequence_manager
            .get_sequence(move_seq_id)
            .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
            .unwrap_or(false);
        let final_entity_seek_arrival = if is_final_waypoint_after_transition_cleanup
            && movement_is_last_sequence_element
            && ft.target_id.is_some()
        {
            live_seek_target.map(|(target_position, target_sector, _)| {
                let same_sector =
                    target_sector.is_some() && target_sector == entity.element_data().sector();
                let target_unchanged = target_position == ft.last_seek_target_position;
                same_sector && (target_unchanged || tolerance_arrival)
            })
        } else {
            None
        };
        if final_entity_seek_arrival == Some(false) {
            // Seeking reports this frame as still in progress
            // once it decides to refresh, so the Execute arm never
            // reaches the switch that would retire the actor into
            // its waiting state. Drop the effect the terminating
            // transition queued a moment ago; leaving it applied
            // strands the actor at a standstill, and the refresh
            // then reads that as a walk rather than the run it was
            // already doing.
            deferred
                .movement_state_effects
                .retain(|(id, _, _)| *id != eid);
            deferred
                .transition_seek_refreshes
                .push((eid, move_seq_id, move_elem_idx));
            return None;
        }
        // Motion through the last frame can mutate the order list
        // before returning TERMINATED: when a startup transition
        // loops short of its destination, it inserts a copied order
        // using the next distinct animation. Seeking then reads
        // that live successor, just like it does after an ordinary
        // walking waypoint, and rejects an out-of-reach stop
        // transition when the seek target moved in the meantime
        // before transition cleanup.
        if !is_final_waypoint_after_transition_cleanup
            && let Some((target_position, _, target_point)) = live_seek_target
            && target_position != ft.last_seek_target_position
            && let Some(next_action) = self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.get(1))
                .map(|order| order.order_type)
            && matches!(
                next_action,
                OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
        {
            let aim = target_point.unwrap_or(target_position);
            let here = entity.element_data().position_map();
            let dx = aim.x - here.x;
            let dy = if ft.directional {
                const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                (aim.y - here.y) * INVERSE_ASPECT_RATIO
            } else {
                aim.y - here.y
            };
            let reach =
                (f32::from(entity.sprite().distance_for_animation(next_action)) + ft.tol) * 1.05;
            if dx * dx + dy * dy > reach * reach {
                deferred
                    .movement_state_effects
                    .retain(|(id, _, _)| *id != eid);
                deferred
                    .transition_seek_refreshes
                    .push((eid, move_seq_id, move_elem_idx));
                tracing::trace!(
                    ?eid,
                    ?next_action,
                    reach,
                    "tick_move: looped transition exposed stale stop; refreshing seek",
                );
                return None;
            }
        }
        // A Hit can be attached to a Seek whose authored stop
        // transition uses up the last few map units before the
        // interaction.  Original terminates that transition at
        // this boundary, then the HITTING init guard rejects an
        // antagonist still farther than 40 map units away.  The
        // rejected post-seek never becomes the actor's visible
        // command at the frame boundary (Nescafe save controls:
        // 55.8 and 41.4 units respectively).  Rust previously
        // instructed the Hit during this same movement drain,
        // exposing one spurious HitCmd frame before its ordinary
        // next-Execute validity guard rejected it.
        let terminal_interaction = entity
            .is_pc()
            .then(|| actor_post_seek_interaction(entity.actor_data().expect("actor-only branch")))
            .flatten();
        let terminal_interaction_out_of_range = final_entity_seek_arrival == Some(true)
            // HITTING's rejected initialization is collapsed at this
            // boundary by the controls above. TYING is different:
            // The original game publishes the newly instructed tying order as
            // IN_PROGRESS first, and its Execute-time position check
            // cannot run until the actor's following update.
            && terminal_interaction == Some(ActorPostSeekInteraction::Hit)
            && live_seek_target
                .map(|(target_position, _, _)| {
                    let here = entity.element_data().position_map();
                    interaction_exceeds_init_range(here, target_position)
                })
                .unwrap_or(false);
        if terminal_interaction_out_of_range {
            // HITTING initialization turns toward its antagonist
            // before the validity check which aborts it
            // before the validity check which aborts it.
            let target_ground = live_seek_target_ground
                .expect("terminal entity seek retained its target ground position");
            let here_ground = entity.ground_position();
            let facing = vector_to_sector_0_to_15(
                target_ground.x - here_ground.x,
                target_ground.y - here_ground.y,
            );
            entity.element_data_mut().set_direction_goal(facing);

            // PC-only: these replay controls use the PC Hit arm,
            // whose invalid interaction has no NPC Think/AI
            // continuation. Keep NPC post-seek lifecycle on the
            // ordinary sequence-manager path.
            let actor = entity.actor_data_mut().expect("actor-only branch");
            // Post-seek sequence launch clears the seek ownership and
            // folds its overloaded wait scalar before HITTING is
            // instructed; the later ABORTED result does not
            // restore any of it. Mirror that pre-abort teardown.
            actor.abort_out_of_range_hit_seek();
            deferred.order_pops.push((move_seq_id, move_elem_idx));
            return None;
        }
        if final_entity_seek_arrival == Some(true) {
            let actor = entity.actor_data_mut().expect("actor-only branch");
            if actor.post_seek_sequence.is_some() && actor.active_door_pass.is_none() {
                deferred
                    .post_seek_arrivals
                    .push((eid, move_seq_id, move_elem_idx));
                actor.clear_path();
                actor.active_movement.clear();
                actor.active_door_pass = None;
            } else {
                // No action consumes the arrival yet. Match
                // seeking's frozen refresh arm rather than
                // exhausting the final transition order.
                actor.seek_refresh_wait = 0;
            }
            return None;
        }
        Some(TransitionSeekCleanup {
            is_final_waypoint_after_transition_cleanup,
            movement_is_last_sequence_element,
        })
    }

    /// Point-seek and actor-owned entity-seek post-seek handoff after a
    /// terminated transition. Returns `true` when the Execute is complete.
    fn hand_off_actor_owned_post_seek(&mut self, cleanup: TransitionSeekCleanup) -> bool {
        let SelectedMovementOrder {
            is_final_waypoint,
            move_seq_id,
            move_elem_idx,
            ..
        } = self.order;
        let TransitionSeekCleanup {
            is_final_waypoint_after_transition_cleanup,
            movement_is_last_sequence_element,
        } = cleanup;
        let MovementStepEntry {
            actor_seek_flags,
            live_actor_seek_target,
            ..
        } = self.entry;
        let ft = self.prepass.final_tolerance;
        let prepass = self.prepass;
        let entity = &mut *self.entity;
        let deferred = &mut *self.deferred;
        let eid = self.entity_id;
        // Point-target Seek reaches this early transition arm
        // after its authored stop transition terminates. Original
        // only starts the post-seek when the seek sector still equals
        // the actor's sector; a player Stop can terminate the
        // transition short of that sector.
        // Retiring it through the ordinary order-pop path first
        // creates a fallback Wait and leaves the post-seek action
        // stranded on ActorData for one frame (or forever).
        let final_point_post_seek_arrival = is_final_waypoint_after_transition_cleanup
            && ft.target_id.is_none()
            && prepass
                .point_seek_post_sector
                .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                .unwrap_or(false)
            && entity
                .actor_data()
                .is_some_and(|actor| actor.post_seek_sequence.is_some())
            && entity
                .actor_data()
                .is_some_and(|actor| actor.active_door_pass.is_none());
        // A copied terminal stop transition can lose the movement
        // element's target while actor seek handling still owns the
        // entity in the seek-target reference. That remains entity-seek mode, not
        // point-seek mode: on the last order Original starts the
        // post-seek when the target is still in our sector and either
        // has not moved since seek refresh or is inside tolerance.
        let final_actor_entity_post_seek_arrival = is_final_waypoint
            && movement_is_last_sequence_element
            && ft.target_id.is_none()
            && actor_seek_flags.contains(MoveFlags::SEEK)
            && live_actor_seek_target.is_some_and(
                |(_, _, target_sector, target_unchanged_or_in_tolerance)| {
                    target_sector.is_some()
                        && target_sector == entity.element_data().sector()
                        && target_unchanged_or_in_tolerance
                },
            )
            && entity
                .actor_data()
                .is_some_and(|actor| actor.post_seek_sequence.is_some())
            && entity
                .actor_data()
                .is_some_and(|actor| actor.active_door_pass.is_none());
        let final_actor_owned_post_seek_arrival =
            final_point_post_seek_arrival || final_actor_entity_post_seek_arrival;
        let actor_owned_interaction = entity
            .is_pc()
            .then(|| actor_post_seek_interaction(entity.actor_data().expect("actor-only branch")))
            .flatten();
        let actor_owned_interaction_out_of_range = final_actor_owned_post_seek_arrival
            && actor_owned_interaction == Some(ActorPostSeekInteraction::Hit)
            && live_actor_seek_target
                .map(|(target_position, _, _, _)| {
                    interaction_exceeds_init_range(
                        entity.element_data().position_map(),
                        target_position,
                    )
                })
                .unwrap_or(false);
        if actor_owned_interaction_out_of_range {
            // A copied terminal transition can lose its movement
            // element target while seek handling's target reference
            // remains actor-owned. HITTING still turns before its
            // validity abort.
            let (_, target_ground, _, _) = live_actor_seek_target
                .expect("out-of-range actor-owned Hit retained a live target");
            let here_ground = entity.ground_position();
            let facing = vector_to_sector_0_to_15(
                target_ground.x - here_ground.x,
                target_ground.y - here_ground.y,
            );
            entity.element_data_mut().set_direction_goal(facing);

            let actor = entity.actor_data_mut().expect("actor-only branch");
            actor.abort_out_of_range_hit_seek();
            deferred.order_pops.push((move_seq_id, move_elem_idx));
            return true;
        }
        let actor = entity.actor_data_mut().expect("actor-only branch");
        if final_actor_owned_post_seek_arrival {
            deferred
                .post_seek_arrivals
                .push((eid, move_seq_id, move_elem_idx));
            actor.clear_path();
            actor.active_movement.clear();
            actor.active_door_pass = None;
            return true;
        }
        false
    }

    /// Retire the terminated transition order and, on the final waypoint,
    /// advance or finish the door pass.
    fn retire_terminated_transition(&mut self, discarded_lazy_door_followers: bool) {
        let SelectedMovementOrder {
            goal,
            is_final_waypoint,
            move_seq_id,
            move_elem_idx,
            ..
        } = self.order;
        let is_swordfighting = self.traits.is_swordfighting;
        let deferred = &mut *self.deferred;
        let eid = self.entity_id;
        // Re-borrow of the actor already checked by the post-seek handoff.
        let actor = self.entity.actor_data_mut().expect("actor-only branch");
        // Pop via the element we actually dispatched (`move_seq_id` /
        // `move_elem_idx`), not `actor.active_movement.sequence_id`
        // — the latter can be stale/None when the Move element was
        // launched by the AI without setting active_movement
        // (soldier chase paths).
        deferred.order_pops.push((move_seq_id, move_elem_idx));
        // Last order of the Move element just completed — flip
        // back to Waiting and clear the active movement.
        // Matches the `DoorPassAdvance::Done` arm below but for
        // the transition-terminated path.
        if is_final_waypoint {
            let mut clear_completed_movement_goal = false;
            let advance = if actor.active_door_pass.is_some() {
                EngineInner::advance_door_pass(actor, eid, goal, &mut self.orders.next_order_id)
            } else {
                DoorPassAdvance::Done { completed: None }
            };
            match advance {
                DoorPassAdvance::Continue {
                    order_id,
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                } => {
                    let mut order =
                        crate::order::Order::new(action, destination.x, destination.y, order_id);
                    order.reverse = reverse;
                    order.compute_direction = compute_direction;
                    order.tolerance = tolerance;
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Paused { transition_order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, transition_order));
                }
                DoorPassAdvance::ActionPoint { order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Done { completed } => {
                    if let Some((door_index, direct)) =
                        completed_door_pass_to_commit(discarded_lazy_door_followers, completed)
                    {
                        deferred
                            .completed_door_passes
                            .push((eid, door_index, direct));
                    }
                    actor.clear_path();
                    actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                        crate::element::ActionState::WaitingSword
                    } else {
                        crate::element::ActionState::Waiting
                    };
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    clear_completed_movement_goal = true;
                }
                DoorPassAdvance::NoActive => {
                    tracing::warn!(
                        entity = ?eid,
                        "DoorPass: transition-terminated movement lost active pass"
                    );
                }
            }
            if clear_completed_movement_goal {
                deferred.terminal_door_pass_goal_clears.push(eid);
            }
        }
    }

    /// Zero-distance animation ticks are still real seeking /
    /// motion steps. The pre-motion tolerance branch and an
    /// ordinary order whose destination already equals the actor's
    /// position both complete without sprite displacement. In
    /// particular, a freshly initialized exact-position walk returns
    /// TERMINATED on that first execution. Only defer a
    /// genuinely stationary motion that has not reached its goal.
    /// Returns `true` when the Execute is complete.
    pub(super) fn queue_stationary_motion_wait(
        &mut self,
        plan: &MovementMotionPlan,
        effects: &MovementStepEffects,
    ) -> bool {
        if stationary_motion_waits(effects.speed, plan.seek.tolerance_arrival, plan.facing.dist) {
            if let Some((posture, next_action_state)) =
                movement_execute_state_effect(self.order.order_action, effects.state_effect_motion)
            {
                self.deferred.movement_state_effects.push((
                    self.entity_id,
                    posture,
                    next_action_state,
                ));
            }
            return true;
        }
        false
    }

    /// Snapshot the pre-step crossing inputs and handle the frozen seek wait.
    /// `None` means the Execute is complete.
    pub(super) fn prepare_movement_arrival(
        &mut self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
        effects: &MovementStepEffects,
    ) -> Option<MovementArrivalState> {
        let SelectedMovementOrder {
            goal,
            action_state,
            is_final_waypoint,
            order_action,
            ..
        } = self.order;
        let MovementMotionPlan {
            seek: MovementSeekEntry {
                tolerance_arrival, ..
            },
            facing: MovementCombatFacing { dist, .. },
            anim,
            ..
        } = *plan;
        let speed = effects.speed;
        let entity_id = self.entity_id;
        let prepass = self.prepass;
        let human_is_carried = self.traits.human_is_carried;
        let entity = &mut *self.entity;
        let elem = entity.element_data_mut();
        tracing::trace!(
            "tick_move: entity={:?} pos=({:.0},{:.0}) goal=({:.0},{:.0}) speed={speed:.1} action={:?} state={:?}",
            entity_id,
            elem.position_map().x,
            elem.position_map().y,
            goal.x,
            goal.y,
            anim,
            action_state,
        );

        // Snapshot the pre-move position + layer + posture so we
        // can run the elevation-line-crossing check after the
        // position is updated. Original excludes flying actors and
        // humans with a live carrier; wall/ladder climbers and actors
        // carrying somebody else remain eligible.
        let old_pos = elem.position_map();
        // The actor update snapshots position before execution and uses that
        // outer position for line-crossing checks after execution returns.
        // RunningStairs performs two literal motion commits inside Execute;
        // by this point `old_pos` is already the post-first-commit position.
        // Retain the outer snapshot so a bond crossed only by the first
        // substep is not lost.
        let crossing_old_pos = if step.first_fast_commit.is_some() {
            step.fast_motion_outer_pre
        } else {
            old_pos
        };
        let entity_layer = elem.layer();
        let entity_posture = elem.posture();
        let eligible_for_crossing = actor_line_crossing_eligible(
            entity_posture,
            human_is_carried,
            self.fast_grid
                .level
                .map_bbox
                .contains_point(crossing_old_pos),
        );

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
        let ft = prepass.final_tolerance;
        let point_seek_post_arrival = is_final_waypoint
            && dist <= speed
            && prepass
                .point_seek_post_sector
                .map(|seek_sector| elem.sector() == Some(seek_sector))
                .unwrap_or(false);
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
                entity = ?entity_id,
                "tick_move: FROZEN seek wait (target in range, no post-seek, mid-path)",
            );
            refresh_pc_walking_shield_after_execute(
                entity,
                &self.assets.profile_manager,
                order_action,
            );
            return None;
        }
        Some(MovementArrivalState {
            crossing_old_pos,
            entity_layer,
            entity_posture,
            eligible_for_crossing,
            point_seek_post_arrival,
        })
    }

    /// Original-game entity-target seek movement samples its live-target
    /// tolerance before motion processing. If already in range it takes
    /// the frozen/post-seek arm without committing a movement step.
    /// If this frame's step merely crosses into range, that new
    /// distance is not sampled until the next actor update.
    ///
    /// Ordinary waypoint arrival is different: motion processing commits
    /// through collision-aware position updates and then checks
    /// goal arrival. Re-enter the shared tail after that commit while
    /// retaining (not recomputing) the pre-motion seek predicate.
    ///
    /// `Break` means the Execute is complete; `Continue` carries whether the
    /// arrival already queued the line crossings.
    pub(super) fn run_movement_arrival_loop(
        &mut self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
        effects: &MovementStepEffects,
        arrival: &mut MovementArrivalState,
    ) -> std::ops::ControlFlow<(), bool> {
        let is_final_waypoint = self.order.is_final_waypoint;
        let MovementMotionPlan {
            seek: MovementSeekEntry {
                tolerance_arrival, ..
            },
            facing:
                MovementCombatFacing {
                    dist,
                    is_sword_motion,
                    ..
                },
            ..
        } = *plan;
        let MovementStepEffects {
            speed,
            split_motion_speeds,
            ..
        } = *effects;
        let entity_id = self.entity_id;
        let actor_id = self.actor_id;
        let prepass = self.prepass;
        let provenance_frame = self.entry.provenance_frame;
        let live_seek_target = self.entry.live_seek_target;
        let mut post_step_arrival = dist <= f32::EPSILON || tolerance_arrival;
        let mut arrived_after_committed_step = false;
        let mut arrival_crossing_queued = false;
        'arrival: loop {
            if post_step_arrival {
                match EngineInner::settle_movement_waypoint(
                    self.entity,
                    self.orders,
                    self.assets,
                    prepass,
                    self.order,
                    entity_id,
                    MovementArrivalBoundary {
                        tolerance_arrival,
                        point_seek_post_arrival: arrival.point_seek_post_arrival,
                        arrived_after_committed_step,
                        crossing_old_pos: arrival.crossing_old_pos,
                        entity_layer: arrival.entity_layer,
                        eligible_for_crossing: arrival.eligible_for_crossing,
                        is_sword_motion,
                        live_seek_target,
                    },
                    self.deferred,
                ) {
                    std::ops::ControlFlow::Break(()) => return std::ops::ControlFlow::Break(()),
                    std::ops::ControlFlow::Continue(crossing_queued) => {
                        arrival_crossing_queued |= crossing_queued;
                        break 'arrival;
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
                // Motion processing advances with the position interface's
                // cached map increment. It does not renormalize the
                // remaining goal vector each frame; that cache is rebuilt
                // only when a new order starts or anti-collision changes
                // deviation state. Recomputing here introduced tiny drift
                // into patrol-chief history and eventually flipped exact
                // transition-arrival dot products.
                let entity = &mut *self.entity;
                let cached_increment = entity.position_iface().get_increment_map();
                let anti_on = entity.position_iface().is_anti_collision_on();
                let diagnostic_pre = OrdinaryStepDiagnosticPre {
                    pre_position: if step.first_fast_commit.is_some() {
                        step.fast_motion_outer_pre
                    } else {
                        entity.element_data().position_map()
                    },
                    old_position: entity.position_iface().old_map_position(),
                    deviated_before: entity.position_iface().is_deviated(),
                    blocked_count_before: entity.position_iface().blocked_count,
                    cached_increment,
                    anti_on,
                };
                let movement_aborted = EngineInner::commit_ordinary_movement_step(
                    entity,
                    self.order,
                    MovementStepOperands {
                        actor_id,
                        provenance_frame,
                        speed,
                        split_motion_speeds,
                        first_step_committed: step.first_fast_commit.is_some(),
                        cached_increment,
                        anti_on,
                    },
                    self.anti_snapshots,
                    self.repulsive_points,
                    self.prepared,
                    self.fast_grid,
                    self.deferred,
                );

                if movement_aborted {
                    break 'arrival;
                }

                // Collision-aware position updates have now committed the
                // ordinary frame and rebuilt the increment when deviation
                // changed. This is the exact point where goal completion is checked.
                let movement_goal_reached = self
                    .entity
                    .position_iface()
                    .is_goal_reached(self.fast_grid, prepass.goal_target_info);
                self.record_ordinary_movement_step_diagnostic(
                    plan,
                    step,
                    effects,
                    diagnostic_pre,
                    movement_goal_reached,
                );
                arrival.point_seek_post_arrival = is_final_waypoint
                    && movement_goal_reached
                    && prepass
                        .point_seek_post_sector
                        .map(|seek_sector| self.entity.element_data().sector() == Some(seek_sector))
                        .unwrap_or(false);
                post_step_arrival = movement_goal_reached || tolerance_arrival;
                if post_step_arrival {
                    arrived_after_committed_step = true;
                    continue 'arrival;
                }
                break 'arrival;
            }
        }
        std::ops::ControlFlow::Continue(arrival_crossing_queued)
    }

    /// Record the committed ordinary step for parity movement diagnostics.
    fn record_ordinary_movement_step_diagnostic(
        &self,
        plan: &MovementMotionPlan,
        step: &MovementMotionStep,
        effects: &MovementStepEffects,
        pre: OrdinaryStepDiagnosticPre,
        movement_goal_reached: bool,
    ) {
        let SelectedMovementOrder {
            goal,
            order_action,
            order_tolerance,
            ..
        } = self.order;
        let MovementMotionPlan {
            anim,
            speed_factor,
            apply_speed_factor,
            motion_method,
            ..
        } = *plan;
        let MovementMotionStep {
            frame_dist_raw,
            first_frame_dist_raw,
            first_fast_commit,
            second_fast_operands,
            second_frame_dist_raw,
            ..
        } = *step;
        let MovementStepEffects {
            direction_differs_from_goal,
            speed,
            split_motion_speeds,
            ..
        } = *effects;
        let OrdinaryStepDiagnosticPre {
            pre_position: movement_diag_pre_position,
            old_position: movement_diag_old_position,
            deviated_before: movement_diag_deviated_before,
            blocked_count_before: movement_diag_blocked_count_before,
            cached_increment,
            anti_on,
        } = pre;
        let nx = cached_increment.x;
        let ny = cached_increment.y;
        let entity_id = self.entity_id;
        let entity = &*self.entity;
        let movement_diag_raw_post = entity.element_data().position_map();
        // Motion processing snaps an undeviated zero-tolerance arrival
        // after goal-arrival testing. Include that authoritative visible
        // result in the diagnostic while retaining the raw
        // anti-collision commit separately.
        let movement_diag_post = if movement_goal_reached
            && order_tolerance == 0.0
            && !entity.position_iface().is_deviated()
        {
            goal
        } else {
            movement_diag_raw_post
        };
        let movement_diag_split_calls =
            if crate::movement_diagnostics::parity_movement_capture_active() {
                split_motion_speeds.map_or_else(Vec::new, |(_, second_speed)| {
                    let mut calls = Vec::with_capacity(2);
                    if let Some((first_pre, first_increment, first_speed, first_post)) =
                        first_fast_commit
                    {
                        calls.push(crate::movement_diagnostics::ParityMovementCall {
                            frame_distance_raw: first_frame_dist_raw.into(),
                            effective_distance: first_speed.into(),
                            pre_position: first_pre.into(),
                            requested_delta: MapVec::new(
                                first_increment.x * first_speed,
                                first_increment.y * first_speed,
                            )
                            .into(),
                            post_position: first_post.into(),
                        });
                    }
                    let (second_pre, second_increment) = second_fast_operands
                        .expect("split motion requires captured second-call operands");
                    calls.push(crate::movement_diagnostics::ParityMovementCall {
                        frame_distance_raw: second_frame_dist_raw
                            .expect("split speeds require a second motion distance")
                            .into(),
                        effective_distance: second_speed.into(),
                        pre_position: second_pre.into(),
                        requested_delta: MapVec::new(
                            second_increment.x * second_speed,
                            second_increment.y * second_speed,
                        )
                        .into(),
                        post_position: movement_diag_raw_post.into(),
                    });
                    calls
                })
            } else {
                Vec::new()
            };
        crate::movement_diagnostics::record_parity_movement_step(
            crate::movement_diagnostics::ParityMovementStep {
                entity: entity_id,
                order_action: format!("{order_action:?}"),
                animation: format!("{anim:?}"),
                motion_method: format!("{motion_method:?}"),
                pre_position: movement_diag_pre_position.into(),
                old_position: movement_diag_old_position.into(),
                goal: goal.into(),
                cached_increment: cached_increment.into(),
                frame_distance_raw: frame_dist_raw.into(),
                speed_factor: speed_factor.into(),
                speed_factor_applied: apply_speed_factor,
                direction_differs_from_goal,
                effective_distance: speed.into(),
                anti_collision_on: anti_on,
                deviated_before: movement_diag_deviated_before,
                blocked_count_before: movement_diag_blocked_count_before,
                requested_delta: crate::coordinates::MapVec::new(nx * speed, ny * speed).into(),
                raw_committed_delta: (movement_diag_raw_post - movement_diag_pre_position).into(),
                committed_delta: (movement_diag_post - movement_diag_pre_position).into(),
                post_position: movement_diag_post.into(),
                deviated_after: entity.position_iface().is_deviated(),
                blocked_count_after: entity.position_iface().blocked_count,
                goal_reached_after_commit: movement_goal_reached,
                split_calls: movement_diag_split_calls,
            },
        );
    }

    /// Refresh the shield, queue this step's line crossings and publish the
    /// START state effects that survive the committed step.
    pub(super) fn publish_movement_start_effects(
        &mut self,
        plan: &MovementMotionPlan,
        effects: &MovementStepEffects,
        arrival: &MovementArrivalState,
        arrival_crossing_queued: bool,
    ) {
        let SelectedMovementOrder {
            order_id,
            order_action,
            move_seq_id,
            move_elem_idx,
            ..
        } = self.order;
        let executes_sword_movement = plan.facing.executes_sword_movement;
        let MovementStepEffects {
            state_effect_motion,
            deferred_movement_state_start_due,
            transition_distance_first_execute_due,
            ..
        } = *effects;
        let MovementArrivalState {
            crossing_old_pos,
            entity_layer,
            entity_posture,
            eligible_for_crossing,
            ..
        } = *arrival;
        let entity_id = self.entity_id;
        let human_is_carried = self.traits.human_is_carried;
        let entity = &mut *self.entity;
        let deferred = &mut *self.deferred;
        // Player-character movement updates the retained shield after
        // every shield-walking seek or motion step,
        // including a tolerance-arrival frame with no displacement.
        refresh_pc_walking_shield_after_execute(entity, &self.assets.profile_manager, order_action);

        // Queue an elevation-line-cross check for this tick. The
        // actual fast-grid query + obstacle swap runs after the
        // loop, since `check_for_line_crossing` needs `&mut self`.
        //
        // Also queue a patch-line-cross check for PC actors —
        // LINE_PATCH handling is gated to PCs only.
        let new_pos = entity.element_data().position_map();
        let new_position_in_bounds = self.fast_grid.level.map_bbox.contains_point(new_pos);
        tracing::trace!(
            target: "robin_engine::elevation_crossing",
            ?entity_id,
            eligible_for_crossing,
            new_position_in_bounds,
            posture = ?entity_posture,
            human_is_carried,
            layer = entity_layer,
            old_x = crossing_old_pos.x,
            old_y = crossing_old_pos.y,
            new_x = new_pos.x,
            new_y = new_pos.y,
            "considered queuing elevation crossing"
        );
        if eligible_for_crossing && !arrival_crossing_queued {
            deferred
                .line_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
            deferred
                .non_elevation_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
        }
        // Order pops are drained after all actors so the current order is
        // still physically at the front here. Treat an already-queued
        // pop as a completed Execute when deciding whether a deferred
        // START survives this actor slot.
        let current_order_will_advance = deferred
            .order_pops
            .iter()
            .any(|&(seq_id, elem_idx)| seq_id == move_seq_id && elem_idx == move_elem_idx);
        // Ordinary walking START effects have the same survival rule as
        // generated transition-distance and deferred PC successors.
        // The original game moves first and only then returns its
        // final motion state to Execute; when anti-collision deviation
        // lands inside the goal predicate on that first call, Execute
        // observes TERMINATED and must not briefly enter Moving.
        if matches!(state_effect_motion, MotionState::Start)
            && !deferred_movement_state_start_due
            && !transition_distance_first_execute_due
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // The authored successor owns the deferred movement START only
        // if it remains current after this Execute. A very short
        // successor can complete and hand off to its stop transition in
        // the same call; Original retains Waiting in that case. The
        // Execute switch still only reacts to the motion state it is
        // handed, so a successor whose START the seek wrapper swallowed
        // owns no state effect to postpone.
        if deferred_movement_state_start_due
            && matches!(state_effect_motion, MotionState::Start)
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // A generated transition-distance copy reports START when first
        // booked, but its movement state is authoritative only if that
        // copied order remains current after the Execute. A short copy
        // may satisfy its arrival predicate and hand off in the same
        // call; Original retains the transition's Waiting state for that
        // frame. This survival rule applies to PCs too; their separate
        // deferred-successor marker covers the later authored order.
        if transition_distance_first_execute_due
            && matches!(state_effect_motion, MotionState::Start)
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
    }
}
