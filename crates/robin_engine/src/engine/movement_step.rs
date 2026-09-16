//! Direct movement execution with scoped owner borrows and synchronous effects.
//! Entry operands survive callbacks only where execution needs their earlier values.

#[cfg(test)]
#[path = "movement_step/tests.rs"]
mod tests;
use super::*;
use crate::coordinates::GroundPoint;
use crate::position_interface::SectorHandle;
use crate::sequence::MoveFlags;

/// `(position, sector, current gameplay point)` of the FinalTol seek target.
type LiveSeekTarget = Option<(MapPoint, Option<SectorHandle>, Option<MapPoint>)>;
/// `(position, ground, sector, unchanged_or_in_tolerance)` of the actor-owned
/// seek target.
type LiveActorSeekTarget = Option<(MapPoint, GroundPoint, Option<SectorHandle>, bool)>;

/// Seek operands sampled before motion. The initial distance predicate is reused
/// after motion when deciding whether an exhausted seek can hand off.
#[derive(Clone, Copy)]
pub(super) struct MovementSeekOperands {
    live_seek_target: LiveSeekTarget,
    live_seek_target_ground: Option<GroundPoint>,
    live_actor_seek_target: LiveActorSeekTarget,
}

/// Order-list facts re-read after terminal transition cleanup.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct TransitionSeekCleanup {
    is_final_waypoint_after_transition_cleanup: bool,
    movement_is_last_sequence_element: bool,
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
    pub(super) fn movement_seek_operands(
        &self,
        entity_id: EntityId,
        selected: MovementOwnerSelection,
        ft: FinalTol,
    ) -> MovementSeekOperands {
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
        let actor_seek_flags = self.orders
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
        MovementSeekOperands {
            live_seek_target,
            live_seek_target_ground,
            live_actor_seek_target,
        }
    }

    pub(super) fn tick_one_movement_actor(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        mut selected: MovementOwnerSelection,
        actor_id: crate::entity_id::ActorId,
        mut ft: FinalTol,
    ) -> Option<MotionState> {
        let mut seek_operands = self.movement_seek_operands(entity_id, selected, ft);
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared at Execute entry");
        super::animation::direction_provenance_snapshot(
            entity.position_iface(),
            entity_id,
            self.control.frame_counter,
            "movement_execute_entry",
        );
        let is_pc = entity.is_pc();
        let is_drunken_soldier = entity.is_soldier()
            && entity
                .npc_data()
                .and_then(|npc| npc.ai_brain.base())
                .is_some_and(|base| base.blood_alcohol > 0);
        // Check swordfight status before mutable borrows — needed at
        // movement completion to preserve WaitingSword (idle state
        // is derived from the action state machine, not hardcoded
        // Waiting).
        let is_swordfighting = entity
            .human_data()
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);

        // Read movement operands from the selected sequence element's current
        // order. Its order queue owns the route throughout execution.
        let Some(mut selected_order) = EngineInner::prepare_selected_movement_order(
            entity,
            &self.orders.sequence_manager,
            selected,
            entity_id,
            is_swordfighting,
        ) else {
            return None;
        };

        let mut order_compute_direction = selected_order.order_compute_direction;
        if let Some(motion) =
            self.execute_non_sprite_movement_action(sim, assets, entity_id, selected_order)
        {
            return Some(motion);
        }
        let SelectedMovementOrder {
            order_id,
            order_action,
            move_seq_id,
            move_elem_idx,
            ..
        } = selected_order;
        let entity = self.expect_entity(entity_id, "movement owner");
        let execute_order_initialising = entity
            .actor_data()
            .expect("movement owner lost actor initialization state")
            .execute_order_initialising;
        if execute_order_initialising
            && is_pc
            && matches!(
                order_action,
                OrderType::WalkingWithCorpse | OrderType::WalkingCarryingOnShoulders
            )
        {
            let carried = entity
                .pc_data()
                .and_then(|pc| pc.carried)
                .expect("carrying movement has no carried actor");
            self.actor_freeze_execution(sim, assets, carried);
            if order_action == OrderType::WalkingCarryingOnShoulders {
                let sprite = &mut self
                    .world
                    .entities
                    .get_mut(carried)
                    .expect("shoulder rider disappeared during initialization")
                    .element_data_mut()
                    .sprite;
                sprite.force_animation(OrderType::WaitingOnShoulders, 0);
                sprite.reset_sprite_frame(false);
            }
        }
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution");
        let mut tolerance_arrival = perform_seek_calls_per_execute(order_action) > 0
            && seek_tolerance_reached(
                ft,
                seek_operands.live_seek_target,
                entity.element_data().position_map(),
                entity.element_data().sector(),
            );
        let soldier_attentive = matches!(entity, crate::element::Entity::Soldier(_))
            && entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
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
            order_compute_direction = false;
        }

        let SelectedMovementOrder {
            goal,
            action_state,
            door_pass_anim,
            order_action,
            ..
        } = selected_order;
        let (combat_target, combat_face_target_is_ground) = self.combat_face_target_for_owner(
            entity_id,
            executes_shield_movement_action(door_pass_anim, order_action),
        );
        let provenance_frame = self.control.frame_counter;
        let elem = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution")
            .element_data_mut();
        let dx = goal.x - elem.position_map().x;
        let dy = goal.y - elem.position_map().y;
        // Combat movement: face opponent, select directional
        // animation.  `compute_direction=false` (don't auto-face
        // movement direction), face toward opponent, pick
        // forward/backward/strafe animation based on angle between
        // movement vector and facing vector.
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
                let face_origin = if combat_face_target_is_ground {
                    let position = elem.position();
                    crate::coordinates::MapPoint::new(position.x, position.y)
                } else {
                    elem.position_map()
                };
                let fdx = opp_pos.x - face_origin.x;
                let fdy = opp_pos.y - face_origin.y;
                tracing::trace!(
                    entity = ?entity_id,
                    frame = self.control.frame_counter,
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
        // map-space goal every tick. Execution stamps
        // the goal once from its normalized 3D increment (including the
        // live ground plane), then returns early while that increment
        // remains valid. Motion processing below owns that initialization;
        // anti-collision and path-boundary code explicitly invalidate and
        // rebuild it when the trajectory actually changes.

        let anim = {
            let SelectedMovementOrder {
                goal,
                action_state,
                door_pass_anim,
                order_action,
                ..
            } = selected_order;
            let (_, combat_face_target_is_ground) =
                self.combat_face_target_for_owner(entity_id, executes_shield_movement);
            let (lift_translation, _, _) = self.movement_owner_lift(entity_id, selected);
            let elem = self
                .world
                .entities
                .get(entity_id)
                .expect("movement owner disappeared during execution")
                .element_data();
            if let Some(dp_anim) =
                door_pass_anim.filter(|anim| !is_sword_movement_nonanimation(*anim))
            {
                // PassDoor supplies the current translated movement step, but
                // Soldier execution still dispatches that logical action
                // through its attentive-animation override. In particular,
                // an attentive WalkingUpright door step plays
                // WalkingAlerted and therefore uses its distinct frame
                // distances.
                super::animation::soldier_movement_animation(
                    dp_anim,
                    soldier_attentive,
                    action_state,
                )
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
                    // first would lose the degenerate cases execution
                    // resolves through its determinant test.
                    let facing = if let Some(opp_pos) = combat_target {
                        let face_origin = if combat_face_target_is_ground {
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
                        ground_origin = combat_face_target_is_ground,
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
                let base = if !order_uses_distance_motion(order_action)
                    || is_authored_climb_action(base)
                {
                    // Movement-animation selection rewrites the movement
                    // element once when it is instructed. Every path order
                    // retains that authored climb direction, even if a later
                    // waypoint briefly bends the other way.
                    base
                } else {
                    match lift_translation {
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
        };
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
        } = selected_order;
        let (lift_translation, door_pass_climb_direction, decorative_building_trap_at_destination) =
            self.movement_owner_lift(entity_id, selected);
        let speed_factor = self
            .orders
            .sequence_manager
            .get_element(selected_order.move_seq_id, selected_order.move_elem_idx)
            .expect("selected movement element disappeared before motion")
            .speed_factor();
        let command = self
            .orders
            .sequence_manager
            .get_element(selected_order.move_seq_id, selected_order.move_elem_idx)
            .expect("selected movement element disappeared before motion")
            .command;
        // Completion callbacks may replace the sequence before this Execute
        // arm returns; retain its dispatch classification, not the live element.
        let door_transition_has_owner = command == crate::element::Command::PassDoor;
        let provenance_frame = self.control.frame_counter;
        let live_seek_target = seek_operands.live_seek_target;
        let order_compute_direction = order_compute_direction;
        let elem = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution")
            .element_data_mut();
        let mut speed_factor = speed_factor;
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
        // Transition execution has two distinct movement paths.
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
        // tokens: execution executes the ordinary sprite motion
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
        }) = lift_translation
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
            decorative_building_trap_at_destination,
        ) {
            self.publish_entity_order_posture(entity_id, posture);
        }
        let elem = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution")
            .element_data_mut();
        if execute_order_initialising && let Some(climb_dir) = door_pass_climb_direction {
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

        // Fast ladder and wall movement stop after a terminated first call.
        // Running stairs always perform both calls, with each call observing
        // the position, increment and seeking state left by the preceding call.
        // Motion short-circuits a newly initialized non-transition
        // motion only when the destination exactly equals the map position.
        // A near-target continuation must still process motion so its
        // ordinary arrival path snaps and retires it in this owner slot.

        let seeking = active_move_flags.contains(MoveFlags::SEEK)
            && perform_seek_calls_per_execute(order_action) != 0;
        let mut motion_call = 0;
        let mut raw_motion_state;
        let motion_state = loop {
            let speed_factor = if motion_call == 0 {
                speed_factor
            } else {
                let (seq_id, elem_idx) = self
                    .world
                    .entities
                    .current_element_for_actor(entity_id)
                    .expect("repeated movement lost its live sequence element");
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .expect("repeated movement lost its live sequence element")
                    .speed_factor()
            };
            let motion_order = motion_order.map(|mut context| {
                context.order_id = selected.order_id;
                let (seq_id, elem_idx) = self
                    .world
                    .entities
                    .current_element_for_actor(entity_id)
                    .expect("movement lost its live sequence element before motion");
                context.next_destination_same_action = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| element.next_order())
                    .filter(|next| next.order_type == selected_order.order_action)
                    .map(|next| MapPoint::new(next.target_x, next.target_y));
                context
            });
            // Each call samples seeking and commits motion before the next call.
            if seeking && ft.target_id.is_some() && !(tolerance_arrival && ft.has_post_seek) {
                let actor = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .expect("seeking owner disappeared")
                    .actor_data_mut()
                    .expect("seeking owner is an actor");
                actor.wait_time = age_seek_refresh_wait(actor.wait_time);
            }
            let position = self
                .world
                .entities
                .get(entity_id)
                .expect("movement owner disappeared before motion")
                .element_data()
                .position_map();
            let delta = selected_order.goal - position;
            let dist = (delta.x * delta.x + delta.y * delta.y).sqrt();
            let dest_already_at_pos =
                motion_method != MotionMethod::TillLastFrame && position == selected_order.goal;
            let SelectedMovementOrder {
                goal,
                order_action,
                active_move_flags,
                ..
            } = selected_order;
            let provenance_frame = self.control.frame_counter;
            let diagnostic_creation_order =
                crate::sprite::sprite_row_diagnostic_creation_order(provenance_frame, || {
                    self.world.original_creation_order(entity_id)
                });
            let sprite_row_diagnostic = diagnostic_creation_order.is_some();
            let sprite = &mut self
                .world
                .entities
                .get_mut(entity_id)
                .expect("movement owner disappeared during execution")
                .element_data_mut()
                .sprite;
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
            // list, so execution then rotates the actor not at all while its
            // direction goal stays where the door route last left it. Every other
            // branch reaching the block below does turn: ordinary actor execution's
            // movement arms call `Turn()` explicitly in their non-SEEK branch
            // in ordinary movement, seeking calls it in both branches, and danger
            // facing always turns.
            let sword_arm_without_face_turn = executes_sword_movement
                && combat_target.is_none()
                && !active_move_flags.contains(MoveFlags::SEEK);
            // An in-range seek freezes the selected animation and renews the
            // order identity so resuming motion initializes its trajectory.
            // Its Execute result remains in-progress even if the action starts.
            let (motion_state, frame_dist_raw) = if tolerance_arrival {
                if !ft.has_post_seek {
                    sprite.perform_action(
                        sim,
                        Some(selected.order_id),
                        sprite_motion_order_for_nonanimation(anim),
                        u16::from(sprite.position_iface.get_direction().as_u8()),
                        FrameProgression::Frozen,
                        false,
                    );
                    let (element, next_order_id) = self
                        .orders
                        .element_with_order_ids_mut(selected.seq_id, selected.elem_idx)
                        .expect("frozen seek lost its selected element");
                    let order = element
                        .orders
                        .front_mut()
                        .expect("frozen seek lost its selected order");
                    assert_eq!(order.order_id, selected.order_id);
                    order.reseed_id(crate::order::alloc_order_id(next_order_id));
                    selected.order_id = order.order_id;
                    selected_order.order_id = Some(order.order_id);
                }
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
                let diagnostic_pre =
                    sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
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
                // Seeking owns the Execute result independently of its frozen
                // action's start/done state.
                sprite.last_motion_state = Some(motion_state);
            }

            raw_motion_state = motion_state;
            let motion_state = raw_motion_state;
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
                ..
            } = selected_order;
            let entity = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("movement owner disappeared during execution");
            let sprite = &mut entity.element_data_mut().sprite;
            // Motion processing applies the sequence speed factor before its
            // turn slowdown and 0.7-unit minimum. The order is observable:
            // a slow patrol member with raw distance 2 and factor ~0.58 is
            // clamped to exactly 0.7 after the 0.6 multiplier, rather than
            // scaling an already-clamped 0.7 back below the minimum.
            //
            // Motion processing initializes a new order's direction goal after
            // the caller's turn above. The movement slowdown test
            // happens later and reads the now-live direction/goal pair, so it
            // applies even though the pre-initialization Turn was a no-op.
            let direction_differs_from_goal =
                sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
            // Direct transitions run motion until the last frame without a speed
            // factor. Seeking transitions pass a speed factor to their motion step.
            let speed = scaled_motion_distance(
                frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                direction_differs_from_goal,
            );
            // The selected action decides whether seeking wraps sprite motion.
            // Ladder and wall orders retain the route's SEEK flag but return their
            // direct motion result, including START and DONE.
            let entity_target_seek = active_move_flags.contains(MoveFlags::SEEK)
                && ft.target_id.is_some()
                && perform_seek_calls_per_execute(order_action) > 0;
            let fallback_motion =
                movement_execute_visible_motion(motion_state, false, entity_target_seek);
            tracing::trace!(
                entity = ?entity_id,
                frame = self.control.frame_counter,
                ?order_action,
                ?motion_state,
                ?fallback_motion,
                action_state = ?action_state,
                sprite_frame = sprite.current_frame,
                sprite_counter = sprite.frame_count,
                sprite_num_frames = sprite.num_frames_for_row(sprite.current_row),
                sprite_wait = sprite.wait_time(sprite.current_row, sprite.current_frame),
                frame_distance_raw = frame_dist_raw,
                speed_factor,
                effective_distance = speed,
                remaining_distance = dist,
                order_tolerance,
                deviated = sprite.position_iface.is_deviated(),
                anti_collision = sprite.position_iface.is_anti_collision_on(),
                goal_x = goal.x,
                goal_y = goal.y,
                increment_x = sprite.position_iface.get_increment_map().x,
                increment_y = sprite.position_iface.get_increment_map().y,
                "movement Execute result"
            );
            // Once the copied continuation executes, it no longer anchors the
            // insertion of lazy door steps ahead of its authored successor.
            if transition_distance_continuation {
                let element = self.orders
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
                current_order.transition_distance_continuation = false;
            }

            {
                let SelectedMovementOrder {
                    action_state,
                    door_pass_anim,
                    ..
                } = selected_order;
                let dist = dist;
                let elem = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("movement owner disappeared during execution")
                    .element_data();
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
            let call_motion = if is_transition_anim && !tolerance_arrival {
                'transition: {
                    let goal_reached = {
                        let SelectedMovementOrder {
                            goal,
                            order_action,
                            order_tolerance,
                            order_reverse,
                            next_destination_same_action,
                            ..
                        } = selected_order;
                        let goal_target_info =
                            self.movement_goal_target_info(selected_order.order_antagonist);
                        let provenance_frame = self.control.frame_counter;
                        let (collision_entity, neighbours) = self
                            .world
                            .entities
                            .split_owner(entity_id)
                            .expect("movement owner disappeared during execution");
                        let collision = super::anti_collision::CollisionWorld {
                            neighbours,
                            profiles: &assets.profile_manager,
                        };
                        let mover =
                            super::anti_collision::CollisionMover::new(entity_id, collision_entity);
                        let entity = collision_entity;
                        let transition_has_map_target = goal.x != 0.0 || goal.y != 0.0;
                        if !transition_has_map_target
                            && !is_in_place_movement_transition(order_action)
                        {
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
                        if transition_has_distance {
                            // Match the map increment: motion processing seeded this
                            // normalized vector when the order began and reuses it
                            // unchanged until anti-collision explicitly rebuilds it.
                            let increment = entity.position_iface().get_increment_map();
                            let nx = increment.x;
                            let ny = increment.y;
                            let anti_on = entity.position_iface().is_anti_collision_on();
                            let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                            let (move_box, half_diagonal) = {
                                let pi = entity.position_iface();
                                (*pi.get_move_box(), pi.get_half_diagonal())
                            };
                            let (dx_step, dy_step, deviated, recovered_from_deviation) =
                                if mover.active {
                                    let pi = entity.position_iface_mut();
                                    let was_deviated = pi.is_deviated();
                                    let mut state = super::anti_collision::AntiCollisionState {
                                        pi,
                                        move_box,
                                        half_diagonal,
                                        goal_map,
                                    };
                                    let (dx_step, dy_step) = apply_live_anti_collision_step(
                                        provenance_frame,
                                        &mover,
                                        collision,
                                        &self.ai.global.repulsive_points,
                                        &self.world.fast_grid,
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
                            let mut position = elem.position_map();
                            position.x += dx_step;
                            position.y += dy_step;
                            elem.set_position_map(position);
                            if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                                elem.sprite.position_iface.reset_increment_computed();
                                elem.sprite.position_iface.compute_increment_all(false);
                            } else if recovered_from_deviation {
                                // Motion rebuilds the trajectory even when this
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
                            let recovered_from_deviation =
                                if entity.position_iface().is_anti_collision_on() && mover.active {
                                    let goal_map =
                                        crate::coordinates::MapPoint::new(goal.x, goal.y);
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
                                    let step = apply_live_anti_collision_step(
                                        provenance_frame,
                                        &mover,
                                        collision,
                                        &self.ai.global.repulsive_points,
                                        &self.world.fast_grid,
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
                            refresh_motion_forecast(entity.sprite_mut(), speed);
                        }
                        if transition_has_distance {
                            // The shared motion path refreshes target
                            // leading after every committed transition displacement,
                            // before goal-arrival testing can clear the live increment. A
                            // missing refresh here made arrows aim at the target's
                            // current point during start/stop transitions.
                            refresh_motion_forecast(entity.sprite_mut(), speed);
                        }
                        // TILL_LAST_FRAME still performs the ordinary arrival check
                        // after every nonzero transition step. Reaching the target
                        // zeros both increments and snaps an undeviated zero-tolerance
                        // actor, but the transition keeps playing until its animation
                        // loops unless the next order uses the same animation.
                        let transition_goal_reached = entity
                            .position_iface()
                            .is_goal_reached(&self.world.fast_grid, goal_target_info);
                        let transition_increment_nonzero = {
                            let increment = entity.position_iface().get_increment_map();
                            increment.x != 0.0 || increment.y != 0.0
                        };
                        if transition_goal_reached && speed != 0.0 && transition_increment_nonzero {
                            let should_snap =
                                !entity.position_iface().is_deviated() && order_tolerance == 0.0;
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
                                entity.sprite_mut().compute_display_depth();
                            }
                            if next_destination_same_action.is_some() {
                                raw_motion_state = MotionState::Terminated;
                            }
                        }
                        transition_goal_reached
                    };
                    if matches!(raw_motion_state, MotionState::Terminated) {
                        // TillLastFrame can exhaust its animation before its
                        // distance target is reached (notably the short
                        // Waiting→Walking startup transition). Transition execution does
                        // not discard that remaining distance: it copies the
                        // current order at the first following animation change,
                        // changes the copy to that next animation, then retires
                        // the exhausted transition. This keeps the copied order's
                        // old target as a one-tick continuation.
                        if !goal_reached {
                            self.insert_transition_distance_continuation(selected_order);
                        }
                        let cleanup = match self.hand_off_terminated_transition_seek(
                            sim,
                            assets,
                            entity_id,
                            selected_order,
                            ft,
                            seek_operands,
                            tolerance_arrival,
                        ) {
                            std::ops::ControlFlow::Break(motion) => break 'transition motion,
                            std::ops::ControlFlow::Continue(cleanup) => cleanup,
                        };
                        if let Some(motion) = self.hand_off_actor_owned_post_seek(
                            sim,
                            assets,
                            entity_id,
                            selected_order,
                            ft,
                            seek_operands,
                            cleanup,
                        ) {
                            break 'transition motion;
                        }
                    }
                    movement_execute_visible_motion(raw_motion_state, false, entity_target_seek)
                }
            } else if !stationary_motion_waits(speed, tolerance_arrival, dist) {
                ('ordinary: {
        let Some(mut point_seek_post_arrival) = ('arrival_preparation: {
        let SelectedMovementOrder {
            goal,
            action_state,
            is_final_waypoint,
            order_action,
            ..
        } = selected_order;
        let point_seek_post_sector = self.movement_point_seek_post_sector(entity_id, selected_order);
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution");
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
        let point_seek_post_arrival = is_final_waypoint
            && dist <= speed
            && point_seek_post_sector
                .map(|seek_sector| elem.sector() == Some(seek_sector))
                .unwrap_or(false);
        // An in-range seek without a follow-up waits at any waypoint.
        // Keep the aged refresh timer: only an actual terminal motion
        // arrival resets it and allows an immediate path refresh.
        let frozen_seek_wait = tolerance_arrival && !ft.has_post_seek;
        if frozen_seek_wait {
            tracing::trace!(
                entity = ?entity_id,
                "tick_move: FROZEN seek wait (target in range, no post-seek)",
            );
            refresh_pc_walking_shield_after_execute(
                entity,
                &assets.profile_manager,
                order_action,
            );
            break 'arrival_preparation None;
        }
        Some(point_seek_post_arrival)
        }) else {
            break 'ordinary None;
        };
        let is_final_waypoint = selected_order.is_final_waypoint;
        let point_seek_post_sector = self.movement_point_seek_post_sector(entity_id, selected_order);
        let goal_target_info = self
            .movement_goal_target_info(selected_order.order_antagonist);
        let provenance_frame = self.control.frame_counter;
        let live_seek_target = seek_operands.live_seek_target;
        let mut post_step_arrival = (dist <= f32::EPSILON && speed == 0.0) || tolerance_arrival;
        let mut arrived_after_committed_step = false;
        'arrival: loop {
            if post_step_arrival {
                break 'ordinary Some(self.settle_movement_waypoint(
                    sim,
                    assets,
                    ft,
                    selected_order,
                    entity_id,
                    MovementArrivalBoundary {
                        tolerance_arrival,
                        point_seek_post_arrival: point_seek_post_arrival,
                        arrived_after_committed_step,
                        live_seek_target,
                    },
                ));
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
                let (entity, neighbours) = self
                    .world
                    .entities
                    .split_owner(entity_id)
                    .expect("movement owner disappeared during execution");
                let collision = super::anti_collision::CollisionWorld {
                    neighbours,
                    profiles: &assets.profile_manager,
                };
                let cached_increment = entity.position_iface().get_increment_map();
                let anti_on = entity.position_iface().is_anti_collision_on();
                let diagnostic_pre = OrdinaryStepDiagnosticPre {
                    pre_position: entity.element_data().position_map(),
                    old_position: entity.position_iface().old_map_position(),
                    deviated_before: entity.position_iface().is_deviated(),
                    blocked_count_before: entity.position_iface().blocked_count,
                    cached_increment,
                    anti_on,
                };
                let movement_aborted = EngineInner::commit_ordinary_movement_step(
                    entity,
                    selected_order,
                    actor_id,
                    provenance_frame,
                    speed,
                    collision,
                    &self.ai.global.repulsive_points,
                    &self.world.fast_grid,
                    &mut self.feedback.titbit_manager,
                );

                if movement_aborted {
                    break 'ordinary Some(MotionState::Aborted);
                }

                // Collision-aware position updates have now committed the
                // ordinary frame and rebuilt the increment when deviation
                // changed. This is the exact point where goal completion is checked.
                let movement_goal_reached = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("movement owner disappeared during execution")
                    .position_iface()
                    .is_goal_reached(&self.world.fast_grid, goal_target_info);
                {
        let SelectedMovementOrder {
            goal,
            order_action,
            order_tolerance,
            ..
        } = selected_order;
        let OrdinaryStepDiagnosticPre {
            pre_position: movement_diag_pre_position,
            old_position: movement_diag_old_position,
            deviated_before: movement_diag_deviated_before,
            blocked_count_before: movement_diag_blocked_count_before,
            cached_increment,
            anti_on,
        } = diagnostic_pre;
        let nx = cached_increment.x;
        let ny = cached_increment.y;
        let entity = self
            .world
            .entities
            .get(entity_id)
            .expect("movement owner disappeared during execution");
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
        crate::movement_diagnostics::record_parity_movement_step(
            || crate::movement_diagnostics::ParityMovementStep {
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
            },
        );
                }
                point_seek_post_arrival = is_final_waypoint
                    && movement_goal_reached
                    && point_seek_post_sector
                        .map(|seek_sector| {
                            self
                                .world
                                .entities
                                .get(entity_id)
                                .expect("movement owner disappeared during execution")
                                .element_data()
                                .sector()
                                == Some(seek_sector)
                        })
                        .unwrap_or(false);
                post_step_arrival = movement_goal_reached || tolerance_arrival;
                if post_step_arrival {
                    arrived_after_committed_step = true;
                    continue 'arrival;
                }
                break 'arrival;
            }
        }
        None
        }).unwrap_or(fallback_motion)
            } else {
                fallback_motion
            };
            motion_call += 1;
            if !fast_climb_motion
                || motion_call == 2
                || (fast_climb_stops_after_first_termination
                    && call_motion == MotionState::Terminated)
            {
                break call_motion;
            }
            if seeking {
                let actor = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("seeking owner disappeared between motion calls")
                    .actor_data()
                    .expect("seeking owner is an actor");
                if !actor.continuation.seek_to_point && actor.seek_target.is_none() {
                    break MotionState::Terminated;
                }
                // The previous call may have expired the refresh countdown.
                // Refresh before the next call ages it or advances the sprite.
                if self.tick_refresh_seek_for_owner(sim, assets, entity_id) {
                    break MotionState::InProgress;
                }
                let (seq_id, elem_idx) = self
                    .world
                    .entities
                    .current_element_for_actor(entity_id)
                    .expect("repeated seeking lost its live sequence element");
                let live_order = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|element| element.current_order())
                    .expect("repeated seeking lost its live order");
                let live_selected = MovementOwnerSelection {
                    seq_id,
                    elem_idx,
                    order_id: live_order.order_id,
                };
                assert!(
                    self.orders
                        .sequence_manager
                        .get_element(selected.seq_id, selected.elem_idx)
                        .and_then(|element| element.current_order())
                        .is_some_and(|order| order.order_id == selected.order_id),
                    "repeated motion cannot execute an order removed by a seek callback"
                );
                ft = self.movement_final_tolerance(entity_id, live_selected);
                seek_operands = self.movement_seek_operands(entity_id, live_selected, ft);
                let entity = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("seeking owner disappeared");
                tolerance_arrival = seek_tolerance_reached(
                    ft,
                    seek_operands.live_seek_target,
                    entity.element_data().position_map(),
                    entity.element_data().sector(),
                );
            }
        };
        let mut motion_state = motion_state;
        let motion = 'execute_tail: {
            let SelectedMovementOrder {
                door_pass_anim,
                order_action,
                active_move_flags,
                ..
            } = selected_order;
            if is_pc && order_action == OrderType::WalkingWithCorpse {
                crate::abilities::sync_walking_corpse_for_carrier(
                    &mut self.world.entities,
                    &assets.profile_manager,
                    entity_id,
                );
            }
            if is_pc && order_action == OrderType::WalkingCarryingOnShoulders {
                crate::abilities::step_shoulder_rider(sim, &mut self.world.entities, entity_id);
                if !self.check_walking_shoulder_clearance(sim, assets, entity_id) {
                    break 'execute_tail MotionState::Aborted;
                }
            }
            if is_sword_motion {
                self.quit_swordfight_with_far_opponents(sim, assets, entity_id);
            }
            let start_survives = motion_state != MotionState::Start
                || self
                    .orders
                    .sequence_manager
                    .get_element(selected_order.move_seq_id, selected_order.move_elem_idx)
                    .and_then(|element| element.current_order())
                    .is_some_and(|order| Some(order.order_id) == selected_order.order_id);
            if start_survives
                && let Some((posture, action_state)) =
                    movement_execute_state_effect(order_action, motion_state)
            {
                self.set_entity_posture(entity_id, posture);
                let entity = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .expect("movement Execute owner disappeared");
                entity
                    .actor_data_mut()
                    .expect("movement Execute owner must be actor")
                    .action_state = action_state;
            }
            if is_pc
                && order_action == OrderType::WalkingWithCorpse
                && motion_state == MotionState::Start
                && start_survives
            {
                let carried = self
                    .expect_entity(entity_id, "walking corpse carrier")
                    .pc_data()
                    .and_then(|pc| pc.carried)
                    .expect("walking corpse carrier has no body");
                self.set_entity_posture(carried, crate::element::Posture::Carried);
                let body = self
                    .world
                    .entities
                    .get_mut(carried)
                    .expect("carried body disappeared");
                body.actor_data_mut()
                    .expect("carried body must be actor")
                    .action_state = crate::element::ActionState::Waiting;
            }
            if executes_sword_movement && motion_state == MotionState::Start && start_survives {
                self.apply_sword_movement_start_initiative_transfer(entity_id);
            }
            if is_sword_motion && motion_state == MotionState::Terminated {
                let step_back = self
                    .world
                    .entities
                    .current_element_for_actor(entity_id)
                    .and_then(|(seq, elem)| self.orders.sequence_manager.get_element(seq, elem))
                    .is_some_and(|element| {
                        matches!(&element.data,
                        crate::sequence::SequenceElementData::Movement { flags, .. }
                            if flags.contains(MoveFlags::STEP_BACK_IN_COMBAT))
                    });
                self.world
                    .entities
                    .get_mut(entity_id)
                    .and_then(Entity::human_data_mut)
                    .expect("sword movement owner lost human state")
                    .last_motion_was_step_back_in_combat = step_back;
                if self.sword_movement_termination_warrants_provoke(assets, entity_id) {
                    self.launch_sword_movement_termination_provoke(sim, assets, entity_id);
                }
            }
            refresh_pc_walking_shield_after_execute(
                self.world
                    .entities
                    .get_mut(entity_id)
                    .expect("movement Execute owner disappeared"),
                &assets.profile_manager,
                order_action,
            );
            if door_pass_anim.is_some()
                && matches!(motion_state, MotionState::Start)
                && matches!(
                    anim,
                    OrderType::TransitionClimbingLadderUpWaitingCrouched
                        | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
                )
            {
                self.apply_door_pass_transition_start_side_effects(assets, entity_id);
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
                self.apply_door_pass_transition_done_side_effects(assets, entity_id);
            }
            if (is_transition_anim && !tolerance_arrival)
                && !(raw_motion_state == MotionState::Terminated
                    && motion_state == MotionState::InProgress)
            {
                {
                    let order_action = selected_order.order_action;
                    let motion_state = raw_motion_state;
                    let door_transition_state_effect_due =
                        matches!(motion_state, MotionState::Terminated)
                            || matches!(motion_state, MotionState::Done)
                                && matches!(
                    anim,
                    OrderType::TransitionClimbingLadderDownWaitingUpright
                        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                );
                    if door_transition_has_owner
                        && door_transition_state_effect_due
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
                        self.apply_door_pass_transition_completion_side_effects(
                            assets,
                            entity_id,
                            order_action,
                        );
                    }
                }
            }
            if active_move_flags.contains(MoveFlags::RIDER_CHARGE)
                && anim == OrderType::RunningUpright
            {
                let sprite = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("rider Execute owner disappeared")
                    .sprite();
                let frame_count = sprite.num_frames_for_anim(OrderType::RunningUpright);
                let cur = sprite.current_frame;
                if is_galopp_decision_frame(cur, frame_count) {
                    self.dispatch_galopp_loop_event(sim, assets, entity_id);
                }
            }
            if is_pc {
                if is_sword_motion {
                    self.abort_pinched_pc_sword_movement(entity_id, &mut motion_state);
                }
            }
            motion_state
        };
        Some(motion)
    }

    fn movement_point_seek_post_sector(
        &self,
        entity_id: EntityId,
        selected_order: SelectedMovementOrder,
    ) -> Option<SectorHandle> {
        if !selected_order.active_move_flags.contains(MoveFlags::SEEK) {
            return None;
        }
        let actor = self
            .world
            .entities
            .get(entity_id)
            .expect("movement owner disappeared")
            .actor_data()
            .expect("movement owner must be an actor");
        if actor.post_seek_sequence.is_none() || !actor.continuation.seek_to_point {
            return None;
        }
        match actor.continuation.seek_sector {
            Some(crate::actor_state::ActorSeekSector::Position(sector)) => Some(sector),
            _ => None,
        }
    }

    fn execute_non_sprite_movement_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_order: SelectedMovementOrder,
    ) -> Option<MotionState> {
        let order_action = selected_order.order_action;
        if !matches!(order_action, OrderType::Freezing | OrderType::PassingDoor) {
            return None;
        }
        let owner = entity_id;
        self.world
            .entities
            .get_mut(owner)
            .expect("movement owner disappeared")
            .element_data_mut()
            .sprite
            .last_motion_state = non_sprite_movement_motion(order_action);
        if order_action == OrderType::Freezing {
            return Some(MotionState::InProgress);
        }
        self.execute_passing_door_order(sim, assets, owner);
        Some(MotionState::Terminated)
    }

    fn insert_transition_distance_continuation(&mut self, selected_order: SelectedMovementOrder) {
        let Some((element, next_order_id)) = self
            .orders
            .element_with_order_ids_mut(selected_order.move_seq_id, selected_order.move_elem_idx)
        else {
            panic!("terminated movement transition lost its element");
        };
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
        if let Some((insertion, animation)) = next_animation {
            let mut continuation = element.orders.front().unwrap().clone();
            continuation.order_type = animation;
            continuation.transition_distance_continuation = true;
            continuation.reseed_id(crate::order::alloc_order_id(next_order_id));
            element.insert_order(insertion, continuation);
        } else {
            element.orders.truncate(1);
        }
    }

    fn hand_off_terminated_transition_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_order: SelectedMovementOrder,
        ft: FinalTol,
        seek_operands: MovementSeekOperands,
        tolerance_arrival: bool,
    ) -> std::ops::ControlFlow<MotionState, TransitionSeekCleanup> {
        let SelectedMovementOrder {
            move_seq_id,
            move_elem_idx,
            ..
        } = selected_order;
        let tolerance_arrival = tolerance_arrival;
        let MovementSeekOperands {
            live_seek_target,
            live_seek_target_ground,
            ..
        } = seek_operands;
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution");
        let eid = entity_id;
        // Seeking wraps the transition animation too. When
        // the last stop transition terminates, seeking checks
        // the live target before retiring the movement: an
        // unchanged same-sector target completes the seek and
        // starts its actor-owned post-seek interaction.
        //
        // The ordinary walking-arrival path below performs this
        // same check, but transition animations return through
        // this earlier branch and must close the handoff here.
        // Motion through the last frame may have deleted every
        // same-animation follower above after looping short of the
        // current destination. Execution's seeking asks
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
            self.refresh_movement_transition_seek(sim, assets, eid, move_seq_id, move_elem_idx);
            return std::ops::ControlFlow::Break(MotionState::InProgress);
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
                self.refresh_movement_transition_seek(sim, assets, eid, move_seq_id, move_elem_idx);
                tracing::trace!(
                    ?eid,
                    ?next_action,
                    reach,
                    "tick_move: looped transition exposed stale stop; refreshing seek",
                );
                return std::ops::ControlFlow::Break(MotionState::InProgress);
            }
        }
        // A Hit can be attached to a Seek whose authored stop
        // transition uses up the last few map units before the
        // interaction.  Movement terminates that transition at
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
            // Execution publishes the newly instructed tying order as
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
            return std::ops::ControlFlow::Break(MotionState::Terminated);
        }
        if final_entity_seek_arrival == Some(true) {
            let actor = entity.actor_data_mut().expect("actor-only branch");
            if actor.post_seek_sequence.is_some() && selected_order.door_pass_anim.is_none() {
                if self.start_post_seek_sequence(
                    sim,
                    assets,
                    &mut Vec::new(),
                    eid,
                    Some((move_seq_id, move_elem_idx)),
                ) {
                    return std::ops::ControlFlow::Break(MotionState::Terminated);
                }
            } else {
                // No action consumes the arrival yet. Match
                // seeking's frozen refresh arm rather than
                // exhausting the final transition order.
                actor.wait_time = 0;
            }
            return std::ops::ControlFlow::Break(MotionState::InProgress);
        }
        std::ops::ControlFlow::Continue(TransitionSeekCleanup {
            is_final_waypoint_after_transition_cleanup,
            movement_is_last_sequence_element,
        })
    }

    fn hand_off_actor_owned_post_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        selected_order: SelectedMovementOrder,
        ft: FinalTol,
        seek_operands: MovementSeekOperands,
        cleanup: TransitionSeekCleanup,
    ) -> Option<MotionState> {
        let SelectedMovementOrder {
            is_final_waypoint,
            move_seq_id,
            move_elem_idx,
            ..
        } = selected_order;
        let TransitionSeekCleanup {
            is_final_waypoint_after_transition_cleanup,
            movement_is_last_sequence_element,
        } = cleanup;
        let actor_seek_flags = selected_order.active_move_flags;
        let MovementSeekOperands {
            live_actor_seek_target,
            ..
        } = seek_operands;
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement owner disappeared during execution");
        let eid = entity_id;
        // Point-target Seek reaches this early transition arm
        // after its authored stop transition terminates. Execution
        // only starts the post-seek when the seek sector still equals
        // the actor's sector; a player Stop can terminate the
        // transition short of that sector.
        // Retiring it through the ordinary order-pop path first
        // creates a fallback Wait and leaves the post-seek action
        // stranded on ActorData for one frame (or forever).
        let final_point_post_seek_arrival = is_final_waypoint_after_transition_cleanup
            && ft.target_id.is_none()
            && entity.actor_data().is_some_and(|actor| {
                actor.continuation.seek_to_point
                    && matches!(actor.continuation.seek_sector,
                        Some(crate::actor_state::ActorSeekSector::Position(sector))
                            if entity.element_data().sector() == Some(sector))
            })
            && entity
                .actor_data()
                .is_some_and(|actor| actor.post_seek_sequence.is_some())
            && selected_order.door_pass_anim.is_none();
        // A copied terminal stop transition can lose the movement
        // element's target while actor seek handling still owns the
        // entity in the seek-target reference. That remains entity-seek mode, not
        // point-seek mode: on the last order seeking starts the
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
            && selected_order.door_pass_anim.is_none();
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
            return Some(MotionState::Terminated);
        }
        if final_actor_owned_post_seek_arrival {
            return Some(
                if self.start_post_seek_sequence(
                    sim,
                    assets,
                    &mut Vec::new(),
                    eid,
                    Some((move_seq_id, move_elem_idx)),
                ) {
                    MotionState::Terminated
                } else {
                    MotionState::InProgress
                },
            );
        }
        None
    }
}
