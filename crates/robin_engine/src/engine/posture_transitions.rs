//! Posture/speed transition orchestration.
//!
//! Three tightly-coupled flows live here:
//!
//! 1. `actor_make_fast` / `actor_make_slow` / `actor_make_upright` /
//!    `actor_make_crouched` — the actor-level dispatcher that
//!    rewrites the in-progress movement sequence, then re-inserts
//!    transition orders via
//!    [`post_process_path`](EngineInner::post_process_path). When
//!    there is no active sequence, falls back to launching a new
//!    `CROUCH_DOWN` / `CROUCH_UP` sequence.
//!
//! 2. `post_process_path` — computes the right start-posture /
//!    start-action-state / end-transition animations based on the
//!    actor's current posture, action state, and the element's
//!    movement action, then inserts start/end transition orders and
//!    cleans up duplicates.
//!
//! Both flows read the actor's current sprite for animation
//! distances, which is only available on `EngineInner`, so this
//! module sits at the engine layer rather than on `SequenceManager`.
//!
//! 3. `post_process_path_to_line` — the line-goal variant of
//!    `post_process_path`. Rewrites the last order's destination to
//!    the nearest point on the goal line and collapses intermediate
//!    waypoints that are directly reachable from that new goal.
//!    Called from the top of
//!    [`post_process_path`](EngineInner::post_process_path) when the
//!    movement element carries [`MoveFlags::LINE`]. This is live for
//!    table-swordfight and other line-goal moves emitted through
//!    `GoalShape::Line` / `AppendMoveToLineToSequence` equivalents.

use crate::coordinates::MapPoint;
use crate::element::{ActionState, Command, DoorPassStep, EntityId, Posture};
use crate::order::OrderType;
use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceId, SequenceState};
use std::collections::VecDeque;

use super::EngineInner;

/// Apply the Original's `InsertTransitionStart` algorithm to the lazy
/// door-pass tail. The C++ engine stores these sub-orders directly in the
/// movement sequence; the port retains them separately until launch.
fn insert_door_pass_start_transition(
    steps: &mut VecDeque<DoorPassStep>,
    point_start: MapPoint,
    animation_to_replace: OrderType,
    animation_transition: OrderType,
    distance_transition: f32,
) {
    let mut distance_remaining = if distance_transition == 0.0 {
        0.01
    } else {
        distance_transition
    };
    let mut point = point_start;
    let mut index = 0;

    while index < steps.len() {
        let mut insert_destination = None;
        if let DoorPassStep::Walk {
            destination,
            action,
            ..
        } = &mut steps[index]
        {
            if *action == animation_to_replace {
                let movement = *destination - point;
                let norm = movement.length();
                if norm >= distance_remaining {
                    let destination = if norm != 0.0 {
                        point + movement.scale(distance_remaining / norm)
                    } else {
                        point
                    };
                    insert_destination = Some(destination);
                } else {
                    distance_remaining -= norm;
                    *action = animation_transition;
                }
            }

            if *destination != MapPoint::ZERO {
                point = *destination;
            }
        }

        if let Some(destination) = insert_destination {
            steps.insert(
                index,
                DoorPassStep::Walk {
                    destination,
                    action: animation_transition,
                    reverse: false,
                    compute_direction: true,
                    tolerance: 0.0,
                },
            );
            return;
        }
        index += 1;
    }
}

/// Continue Original's `InsertTransitionStart` scan from the materialized
/// prefix into Rust's separately stored door-pass tail.
///
/// Returns the last nonzero destination and the transition distance which
/// the materialized orders did not consume. `None` means the transition was
/// placed completely inside the materialized prefix.
fn door_pass_transition_tail_cursor(
    orders: &VecDeque<crate::order::Order>,
    point_start: MapPoint,
    animation_to_replace: OrderType,
    distance_transition: f32,
) -> Option<(MapPoint, f32)> {
    let mut distance_remaining = if distance_transition == 0.0 {
        0.01
    } else {
        distance_transition
    };
    let mut point = point_start;

    for order in orders {
        let destination = MapPoint::new(order.target_x, order.target_y);
        if order.order_type == animation_to_replace {
            let norm = (destination - point).length();
            if norm >= distance_remaining {
                return None;
            }
            distance_remaining -= norm;
        }
        if destination != MapPoint::ZERO {
            point = destination;
        }
    }

    Some((point, distance_remaining))
}

impl EngineInner {
    fn selected_order_action(&self, entity: EntityId) -> Option<OrderType> {
        self.orders
            .sequence_manager
            .current_order_for_actor(entity)
            .map(|(_, _, order)| order.order_type)
    }

    /// A make_* rewrite retargets the selected order's action in place and
    /// leaves its identity alone. Original's `mpOrder` points at that same
    /// mutated object, so `GetAnimation()` observes the new action
    /// immediately even though the sprite does not execute it until its next
    /// actor slot. Keep Rust's detached installed-order copy and the lazy
    /// door-pass mirror pointed at that rewritten action without publishing a
    /// newly inserted, not-yet-installed order. Call this both before and
    /// after `post_process_path`: ordinary paths insert a new transition ID
    /// that the second call rejects, while loaded PassDoor tails can rewrite
    /// the live legacy order in place during post-processing.
    fn synchronize_rewritten_selected_order(
        &mut self,
        entity: EntityId,
        action_before: Option<OrderType>,
    ) {
        let Some(selected_after) = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity)
            .map(|(_, _, order)| crate::element::InstalledActorOrder {
                order_id: order.order_id,
                order_type: order.order_type,
            })
        else {
            return;
        };
        if action_before == Some(selected_after.order_type) {
            return;
        }
        if let Some(actor) = self.get_entity_mut(entity).and_then(|e| e.actor_data_mut()) {
            if actor
                .installed_order
                .is_some_and(|installed| installed.order_id == selected_after.order_id)
            {
                actor.installed_order = Some(selected_after);
            }
            if let Some(pass) = actor.active_door_pass.as_mut() {
                pass.current_action = selected_after.order_type;
            }
        }
    }

    /// Upgrade walking to running for `entity`.
    pub(crate) fn actor_make_fast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
    ) {
        // Early-out: cannot run when carrying a corpse / on shoulders,
        // or when standing in a motion sector that forces crouched
        // movement.
        let (posture, sector_handle) = match self.get_entity(entity) {
            Some(e) => (e.element_data().posture, e.element_data().sector()),
            None => return,
        };
        if matches!(
            posture,
            Posture::CarryingCorpse | Posture::CarryingOnShoulders
        ) {
            return;
        }
        if let Some(handle) = sector_handle {
            let sector_num = crate::sector::SectorNumber(handle.get() as i16);
            if self.sector_forces_crouch(sector_num) {
                return;
            }
        }

        if let Some(selected_movement) = self.selected_movement_element(entity) {
            let action_before = self.selected_order_action(entity);
            self.orders.sequence_manager.make_fast(entity);
            self.synchronize_rewritten_selected_order(entity, action_before);
            if let Some(pathfinder_index) = self.pending_pathfinder_index(entity, selected_movement)
            {
                self.orders
                    .pending_path_requests
                    .make_fast(entity, pathfinder_index);
                return;
            }
            self.make_active_door_pass_fast(entity);
            self.after_make_rewrite(sim, entity, selected_movement);
            self.synchronize_rewritten_selected_order(entity, action_before);
        } else if self.selected_element(entity).is_some() {
            // Base SequenceElement::MakeFast still recurses into a same-owner
            // following/postponed movement even when the selected element is
            // not itself movement. Only the selected-element movement tail
            // (pathfinder/PostProcessPath) is skipped in that case.
            self.orders.sequence_manager.make_fast(entity);
        }
    }

    /// Downgrade running to walking for `entity`.
    pub(crate) fn actor_make_slow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
    ) {
        if let Some(selected_movement) = self.selected_movement_element(entity) {
            let action_before = self.selected_order_action(entity);
            self.orders.sequence_manager.make_slow(entity);
            self.synchronize_rewritten_selected_order(entity, action_before);
            if let Some(pathfinder_index) = self.pending_pathfinder_index(entity, selected_movement)
            {
                self.orders
                    .pending_path_requests
                    .make_slow(entity, pathfinder_index);
                return;
            }
            self.after_make_rewrite(sim, entity, selected_movement);
            self.synchronize_rewritten_selected_order(entity, action_before);
        } else if self.selected_element(entity).is_some() {
            self.orders.sequence_manager.make_slow(entity);
        }
    }

    fn make_active_door_pass_fast(&mut self, entity: EntityId) {
        let Some(owner) = self.get_entity(entity) else {
            return;
        };
        let position = owner.element_data().position_map();
        let actor = owner.actor_data().expect("movement owner is not an actor");
        let movement_action = self
            .selected_movement_element(entity)
            .and_then(|(seq_id, elem_idx)| {
                self.orders.sequence_manager.get_element(seq_id, elem_idx)
            })
            .and_then(|element| match &element.data {
                SequenceElementData::Movement { action, .. } => Some(*action),
                _ => None,
            });
        let transition = if matches!(
            movement_action,
            Some(OrderType::WalkingWithSword | OrderType::RunningWithSword)
        ) {
            // Original PostProcessPath treats both sword movement tokens as
            // total no-transition arms, irrespective of posture/action state.
            None
        } else {
            match owner.element_data().posture {
                // RHelementactor.cpp:7640-7679 selects the crouched posture
                // transition before considering the action-state transition.
                // This matters when MakeFast rewrites the untranslated tail of a
                // high wall pass while TransitionCrouchingUp is still current:
                // Original inserts crouched-walking -> running, not
                // waiting-upright -> running.
                Posture::Crouched => Some(OrderType::TransitionWalkingCrouchedRunningUpright),
                _ => match actor.action_state {
                    ActionState::Moving => Some(OrderType::TransitionWalkingUprightRunningUpright),
                    ActionState::Waiting | ActionState::Bored => {
                        Some(OrderType::TransitionWaitingUprightRunningUpright)
                    }
                    _ => None,
                },
            }
        };
        // Original keeps the already-materialized PassDoor orders and its
        // untranslated tail in one `mlistOrders`. Continue the same
        // InsertTransitionStart scan across Rust's representation boundary:
        // matching materialized orders consume transition distance, every
        // nonzero destination advances the cursor, and zero-position door
        // action points leave it unchanged.
        let transition_and_tail_cursor = transition.map(|transition| {
            let transition_distance = f32::from(owner.sprite().distance_for_animation(transition));
            let tail_cursor = self
                .selected_movement_element(entity)
                .and_then(|(seq_id, elem_idx)| {
                    self.orders.sequence_manager.get_element(seq_id, elem_idx)
                })
                .and_then(|element| {
                    door_pass_transition_tail_cursor(
                        &element.orders,
                        position,
                        OrderType::RunningUpright,
                        transition_distance,
                    )
                });
            (transition, tail_cursor)
        });
        let Some(actor) = self.get_entity_mut(entity).and_then(|e| e.actor_data_mut()) else {
            return;
        };
        let Some(pass) = actor.active_door_pass.as_mut() else {
            return;
        };

        for step in pass.steps.iter_mut() {
            let DoorPassStep::Walk { action, .. } = step else {
                continue;
            };
            *action = match *action {
                OrderType::WalkingUpright | OrderType::WalkingCrouched => OrderType::RunningUpright,
                OrderType::WalkingWithSword => OrderType::RunningWithSword,
                OrderType::WalkingWithShield => OrderType::RunningUpright,
                other => other,
            };
        }
        let Some((transition, Some((tail_start, distance_remaining)))) = transition_and_tail_cursor
        else {
            return;
        };
        insert_door_pass_start_transition(
            &mut pass.steps,
            tail_start,
            OrderType::RunningUpright,
            transition,
            distance_remaining,
        );
    }

    /// Stand the actor up (rewrite crouched movement orders upright).
    ///
    /// We unconditionally call `make_upright` on the active element
    /// (movement or not), so a pending `CrouchDown` element gets its
    /// command nulled (via the `make_upright_element` `CrouchDown →
    /// Null` branch) before falling through to launch `CROUCH_UP`.
    /// Only when the active element is itself a movement do we skip
    /// the `CROUCH_UP` fallback and run the path-rewrite tail
    /// (`after_make_rewrite`) instead.
    pub(crate) fn actor_make_upright(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
    ) {
        if self.selected_element(entity).is_some() {
            let selected_movement = self.selected_movement_element(entity);
            let action_before = self.selected_order_action(entity);
            self.orders.sequence_manager.make_upright(entity);
            self.synchronize_rewritten_selected_order(entity, action_before);
            if let Some(selected_movement) = selected_movement {
                if let Some(pathfinder_index) =
                    self.pending_pathfinder_index(entity, selected_movement)
                {
                    self.orders
                        .pending_path_requests
                        .make_upright(entity, pathfinder_index);
                    return;
                }
                self.after_make_rewrite(sim, entity, selected_movement);
                self.synchronize_rewritten_selected_order(entity, action_before);
                return;
            }
        }
        // No active element, or active element was non-movement (and
        // its command was just nulled by `make_upright`): launch
        // CROUCH_UP so the actor animates to standing.
        let elem = SequenceElement::new(1, Command::CrouchUp, Some(entity));
        let mut sequence = crate::sequence::Sequence::new();
        sequence.append_element(elem);
        self.launch_sequence(sequence);
    }

    /// Crouch the actor down.
    pub(crate) fn actor_make_crouched(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
    ) {
        if let Some(selected_movement) = self.selected_movement_element(entity) {
            let action_before = self.selected_order_action(entity);
            self.orders.sequence_manager.make_crouched(entity);
            self.synchronize_rewritten_selected_order(entity, action_before);
            if let Some(pathfinder_index) = self.pending_pathfinder_index(entity, selected_movement)
            {
                self.orders
                    .pending_path_requests
                    .make_crouched(entity, pathfinder_index);
                return;
            }
            self.after_make_rewrite(sim, entity, selected_movement);
            self.synchronize_rewritten_selected_order(entity, action_before);
        } else {
            if self.selected_element(entity).is_some() {
                // As in SequenceElement::MakeCrouched, recurse into its linked
                // tail before launching the actor's own posture command.
                self.orders.sequence_manager.make_crouched(entity);
            }
            let elem = SequenceElement::new(1, Command::CrouchDown, Some(entity));
            let mut sequence = crate::sequence::Sequence::new();
            sequence.append_element(elem);
            self.launch_sequence(sequence);
        }
    }

    /// Re-insert transition orders after a `make_*` rewrite so the
    /// queued animation sequence remains well-formed, and re-apply
    /// the drunken-midpoint deviation at the new speed.
    ///
    /// Pending `MoveWaiting` requests are handled by the caller through the
    /// pathfinder request rewrite and never reach this materialized-path tail.
    fn after_make_rewrite(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
        (seq_id, elem_idx): (SequenceId, usize),
    ) {
        self.post_process_path(seq_id, elem_idx);
        // Each re-process call re-applies drunken midpoint deviation
        // at the new speed. The initial deviation is applied at
        // pathfind time (tick.rs); this call re-wobbles the remaining
        // waypoints when a drunken soldier transitions walk ↔ run
        // mid-path.
        self.reapply_drunken_deviation(sim, entity, seq_id, elem_idx);
    }

    /// Re-apply drunken path deviation to a soldier's remaining
    /// waypoints after a speed change.  Uses the actor's current
    /// position as the segment origin and the matching drunken factor
    /// for the new movement animation.
    fn reapply_drunken_deviation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        let Some(ent) = self.get_entity(entity) else {
            return;
        };
        if !ent.is_soldier() {
            return;
        }
        let blood_alcohol = ent
            .npc_data()
            .and_then(|n| n.ai_brain.base())
            .map(|b| b.blood_alcohol)
            .unwrap_or(0);
        if blood_alcohol == 0 {
            return;
        }

        // Skip PassDoor.
        let command = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.command);
        if command == Some(Command::PassDoor) {
            return;
        }

        // Read the movement action and filter to walking / running.
        let action = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match e.data {
                SequenceElementData::Movement { action, .. } => Some(action),
                _ => None,
            });
        let is_running = match action {
            Some(OrderType::RunningUpright) | Some(OrderType::RunningWithSword) => true,
            Some(OrderType::WalkingUpright) | Some(OrderType::WalkingWithSword) => false,
            _ => return,
        };

        // Snapshot remaining path + box + layer.  Only re-wobble the
        // waypoints the actor has not yet reached — segments already
        // crossed are fixed history.  The remaining path is read from
        // the active Move element's walking orders (authoritative,
        // post-refactor).
        let (waypoints, position, half_diag, move_box, layer) = {
            let ent = self.get_entity(entity).unwrap();
            let Some(_) = ent.actor_data() else {
                return;
            };
            let remaining: Vec<crate::coordinates::MapPoint> = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|e| {
                    e.orders
                        .iter()
                        .filter(|o| {
                            matches!(
                                o.order_type,
                                OrderType::WalkingUpright
                                    | OrderType::RunningUpright
                                    | OrderType::WalkingWithSword
                                    | OrderType::RunningWithSword
                            )
                        })
                        .map(|o| crate::coordinates::MapPoint::new(o.target_x, o.target_y))
                        .collect()
                })
                .unwrap_or_default();
            let position = ent.element_data().position_map();
            let layer = ent.element_data().layer();
            let (half_diag, move_box) = {
                let pi = ent.position_iface();
                (pi.get_half_diagonal(), *pi.get_move_box())
            };
            (remaining, position, half_diag, move_box, layer)
        };
        if waypoints.is_empty() {
            return;
        }

        let deviated = crate::engine::tick::apply_drunken_path_deviation(
            sim,
            waypoints,
            position,
            blood_alcohol,
            is_running,
            layer,
            &move_box,
            half_diag,
            &self.world.fast_grid,
        );

        // Rewrite the walking-order targets on the Move element with
        // the new deviated points. C++ drunken post-processing inserts
        // fresh RHOrder copies for deviated points, so the sprite sees
        // a new ulUniqueID and re-runs SetPositionGoalMap /
        // ComputeIncrementAll. Rust rewrites targets in place here;
        // reroll the id for any changed target to preserve that
        // PerformMotion invariant.
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        {
            // Skip any non-walking orders at the front (startup
            // transition or end transition — their geometry is not
            // part of the drunken-rewrite path).  Replace subsequent
            // walking orders' targets with the deviated waypoints.
            let mut dev_iter = deviated.iter();
            let next_order_id = &mut self.orders.next_order_id;
            for order in elem.orders.iter_mut() {
                if matches!(
                    order.order_type,
                    OrderType::WalkingUpright
                        | OrderType::RunningUpright
                        | OrderType::WalkingWithSword
                        | OrderType::RunningWithSword
                ) && let Some(next) = dev_iter.next()
                    && ((order.target_x - next.x).abs() > 0.01
                        || (order.target_y - next.y).abs() > 0.01)
                {
                    order.target_x = next.x;
                    order.target_y = next.y;
                    order.reseed_id(crate::order::alloc_order_id(next_order_id));
                }
            }
        }
    }

    fn translate_lift_posture_movement_action(
        &self,
        sector: crate::position_interface::SectorHandle,
        posture: Posture,
        position: crate::coordinates::MapPoint,
        elem: &SequenceElement,
    ) -> Option<OrderType> {
        let sector =
            super::movement::grid_sector_for_position_handle(&self.world.fast_grid.level, sector)?;
        let lift_type = sector.lift_type?;
        if !matches!(
            (posture, lift_type),
            (Posture::OnWall, crate::sector::LiftType::Wall)
                | (Posture::OnLadder, crate::sector::LiftType::Ladder)
        ) {
            return None;
        }

        let destination = elem.orders.back()?;
        let (pt_low, pt_high) = super::movement::lift_endpoint_points_for_sector(sector);
        let ladder_dx = pt_low.x - pt_high.x;
        let ladder_dy = pt_low.y - pt_high.y;
        let move_dx = destination.target_x - position.x;
        let move_dy = destination.target_y - position.y;
        let going_down = ladder_dx * move_dx + ladder_dy * move_dy >= 0.0;
        Some(lift_type.translate_climb_action(destination.order_type, going_down))
    }

    /// Insert start-posture / start-action-state / end transition
    /// orders on the movement element at `(seq_id, elem_idx)` based on
    /// the owner's current (or post-transition) posture + action
    /// state, then dedupe consecutive duplicate orders.
    ///
    /// Dispatches to [`Self::post_process_path_to_line`] at the top
    /// when the movement element carries [`MoveFlags::LINE`].
    pub(crate) fn post_process_path(&mut self, seq_id: SequenceId, elem_idx: usize) -> bool {
        // Snapshot everything we need from entity/element into locals
        // so we can release the immutable borrow before mutating the
        // sequence element. The element's posture/actionstate-after-
        // transition are read from its stored fields when the element
        // is not currently in progress.
        let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            return false;
        };
        let command = elem.command;
        // Accept `Move`, `MoveOk`, and `PassDoor`. The semantic gate
        // is "this is an active movement element with a resolved
        // path", which our element lifecycle enforces by reaching
        // `post_process_path` only after `find_path` returned a
        // route. Note that the Rust port never reassigns the command
        // to `MoveOk` (flipping would break priority resolution in
        // `element_priority.rs::actor_branch`, which only matches
        // `Command::Move`), but accepting both makes the gate robust
        // to either lifecycle.
        if command != Command::Move && command != Command::MoveOk && command != Command::PassDoor {
            return false;
        }
        let (mut animation_movement, flags, tolerance, owner) = match &elem.data {
            SequenceElementData::Movement {
                action,
                flags,
                tolerance,
                ..
            } => (*action, *flags, *tolerance, elem.owner),
            _ => return false,
        };
        let Some(owner) = owner else {
            return false;
        };

        // Line-goal fork: collapse the path onto its goal line before
        // the transition-insertion logic runs, so any rewrite/deletion
        // of orders is visible to the subsequent
        // InsertTransitionStart/End sites.
        if flags.contains(MoveFlags::LINE) {
            self.post_process_path_to_line(seq_id, elem_idx);
        }

        // Re-read the element after `post_process_path_to_line`
        // potentially mutated its order list.
        let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            return false;
        };
        let state = elem.state;
        let elem_posture_after = elem.posture_after_transition;
        let elem_action_state_after = elem.action_state_after_transition;
        let distance_start_posture_anim = None::<OrderType>;
        let _ = distance_start_posture_anim; // placeholder for clarity

        let (current_posture, current_action_state, position, current_sector) = {
            let Some(entity) = self.get_entity(owner) else {
                return false;
            };
            let ed = entity.element_data();
            // Original PostProcessPath reads the actor only for an element
            // that is already executing. A newly instructed or resumed
            // postponed element instead uses the posture/action snapshot
            // captured by its first Instruct, so time spent behind a blocker
            // cannot invent a stop/start transition.
            let (cur_posture, cur_action_state) = if state == SequenceState::InProgress {
                (
                    ed.posture,
                    entity
                        .actor_data()
                        .map(|a| a.action_state)
                        .unwrap_or_default(),
                )
            } else {
                (elem_posture_after, elem_action_state_after)
            };
            (
                cur_posture,
                cur_action_state,
                ed.position_map(),
                ed.sector(),
            )
        };
        tracing::trace!(
            target: "parity_post_process_path",
            ?owner,
            ?seq_id,
            elem_idx,
            ?state,
            live_action_state = ?self
                .get_entity(owner)
                .and_then(|e| e.actor_data())
                .map(|a| a.action_state),
            ?elem_action_state_after,
            ?elem_posture_after,
            ?current_action_state,
            ?current_posture,
            "PostProcessPath start-transition inputs",
        );

        // ── Decide which transitions to insert ──────────────────
        if matches!(current_posture, Posture::OnWall | Posture::OnLadder)
            && let Some(sector) = current_sector
            && let Some(translated) =
                self.translate_lift_posture_movement_action(sector, current_posture, position, elem)
        {
            animation_movement = translated;
        }

        let (animation_start_posture, animation_start_action_state, animation_end) =
            decide_transitions(
                animation_movement,
                current_posture,
                current_action_state,
                flags,
                // `is_next_movement_or_jump` uses the same-sequence
                // walker; good enough for the end-transition gate.
                self.orders
                    .sequence_manager
                    .is_next_movement_or_jump(seq_id, elem_idx),
            );

        // Capture each transition's animation distance from the sprite
        // so the subsequent InsertTransition calls can be made with the
        // sequence manager borrowed mutably.
        let start_posture_distance = animation_start_posture
            .and_then(|anim| self.sprite_distance_for_animation(owner, anim));
        let start_action_state_distance = animation_start_action_state
            .and_then(|anim| self.sprite_distance_for_animation(owner, anim));
        let end_distance =
            animation_end.and_then(|anim| self.sprite_distance_for_animation(owner, anim));

        let _ = tolerance; // `tolerance` is folded into insert_transition_end internally

        // ── Apply transitions in order ──────────────────────────
        let next_order_id = &mut self.orders.next_order_id;
        let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        else {
            return false;
        };
        if matches!(
            animation_movement,
            OrderType::ClimbingWallUp
                | OrderType::ClimbingWallDown
                | OrderType::ClimbingWallUpFast
                | OrderType::ClimbingWallDownFast
                | OrderType::ClimbingLadderUp
                | OrderType::ClimbingLadderDown
                | OrderType::ClimbingLadderUpFast
                | OrderType::ClimbingLadderDownFast
        ) {
            for order in &mut elem.orders {
                if matches!(
                    order.order_type,
                    OrderType::WalkingUpright | OrderType::RunningUpright
                ) {
                    order.order_type = animation_movement;
                    order.compute_direction = false;
                }
            }
        }
        if tracing::enabled!(tracing::Level::TRACE) {
            let pre: Vec<(std::num::NonZeroU32, crate::order::OrderType, f32, f32)> = elem
                .orders
                .iter()
                .map(|o| (o.order_id, o.order_type, o.target_x, o.target_y))
                .collect();
            tracing::trace!(
                owner = ?owner,
                position_start = ?position,
                ?animation_start_posture,
                ?animation_start_action_state,
                ?animation_end,
                ?start_posture_distance,
                ?start_action_state_distance,
                ?end_distance,
                orders_before = ?pre,
                "post_process_path: pre-insertion state"
            );
        }
        let mut inserted_start_transition = false;
        if let (Some(anim), Some(dist)) = (animation_start_posture, start_posture_distance) {
            inserted_start_transition |= elem.insert_transition_start(
                anim,
                animation_movement,
                dist as f32,
                position,
                next_order_id,
            );
        }
        if let (Some(anim), Some(dist)) =
            (animation_start_action_state, start_action_state_distance)
        {
            inserted_start_transition |= elem.insert_transition_start(
                anim,
                animation_movement,
                dist as f32,
                position,
                next_order_id,
            );
        }
        if let (Some(anim), Some(dist)) = (animation_end, end_distance)
            && !flags.contains(MoveFlags::NO_TRANSITIONS)
        {
            // `insert_transition_end` hard-codes the global
            // `ASPECT_RATIO` ≈ 0.5736 when
            // `MoveFlags::DIRECTIONAL_TOLERANCE` is set, applying an
            // isometric Y-stretch to the gap norm.
            elem.insert_transition_end(
                anim,
                animation_movement,
                dist as f32,
                position,
                crate::position_interface::ASPECT_RATIO,
                next_order_id,
            );
        }

        // Clean up consecutive duplicate orders.
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        {
            elem.cleanup_duplicate_orders();
            if tracing::enabled!(tracing::Level::TRACE) {
                let post: Vec<(std::num::NonZeroU32, crate::order::OrderType, f32, f32)> = elem
                    .orders
                    .iter()
                    .map(|o| (o.order_id, o.order_type, o.target_x, o.target_y))
                    .collect();
                tracing::trace!(
                    owner = ?owner,
                    orders_after = ?post,
                    "post_process_path: post-insertion state"
                );
            }
        }
        inserted_start_transition
    }

    /// Line-goal path collapse. Rewrites the last order's destination
    /// to the nearest point on the movement element's goal line, then
    /// walks backward through the non-transition orders deleting any
    /// intermediate waypoint whose previous order is directly
    /// reachable from the new goal. Bails silently if the new goal
    /// itself is not directly reachable from the source.
    ///
    /// Called from the top of [`Self::post_process_path`] when the
    /// movement element carries [`MoveFlags::LINE`].
    fn post_process_path_to_line(&mut self, seq_id: SequenceId, elem_idx: usize) {
        // ── Snapshot line id, source-of-nearest-point, and
        //    transition-order count from the element.  The "source"
        //    is either the second-to-last order's destination (if
        //    there is more than one non-transition order) or the
        //    actor's current map position.
        let (line_id, num_transition_orders, source_from_prev) = {
            let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
                return;
            };
            let line_id = match &elem.data {
                SequenceElementData::Movement { line_id, .. } => *line_id,
                _ => return,
            };
            let n_orders = elem.orders.len();
            let n_trans = elem.num_transition_orders;
            let source_from_prev = if n_orders.saturating_sub(n_trans) > 1 && n_orders >= 2 {
                let o = &elem.orders[n_orders - 2];
                Some(crate::coordinates::MapPoint::new(o.target_x, o.target_y))
            } else {
                None
            };
            (line_id, n_trans, source_from_prev)
        };

        // Resolve the goal line from the jump-line table.  A null
        // line id is a hard error in principle, but here we log and
        // bail rather than silently substituting a zero line.
        let Some(line_id) = line_id else {
            tracing::warn!(
                ?seq_id,
                elem_idx,
                "post_process_path_to_line: MoveFlags::LINE set but line_id is None; skipping"
            );
            return;
        };
        let Some(line) = self
            .world
            .fast_grid
            .level
            .jump_lines
            .get(usize::from(line_id))
        else {
            tracing::warn!(
                ?seq_id,
                elem_idx,
                ?line_id,
                "post_process_path_to_line: line_id out of range; skipping"
            );
            return;
        };
        let line_a = line.point_a;
        let line_b = line.point_b;
        let line_vec = line.vector();
        let sq_norm = line.square_norm();

        // Fetch actor-side context: layer, half-diagonal, and (as a
        // fallback source point) current map position.
        let (position, layer, half_diagonal) = {
            let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
                return;
            };
            let Some(owner) = elem.owner else {
                return;
            };
            let Some(entity) = self.get_entity(owner) else {
                return;
            };
            let position = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let hd = entity.position_iface().get_half_diagonal();
            (position, layer, hd)
        };
        let pt_source = source_from_prev.unwrap_or(position);

        // Nearest point on the [A, B] segment to `pt_source`: clamp
        // to B if past B along the direction vector, clamp to A if
        // before A, otherwise project.
        let pt_new_goal = {
            let dot_b =
                (pt_source.x - line_b.x) * line_vec.x + (pt_source.y - line_b.y) * line_vec.y;
            if dot_b >= 0.0 {
                line_b
            } else {
                let dot_a =
                    (pt_source.x - line_a.x) * line_vec.x + (pt_source.y - line_a.y) * line_vec.y;
                if dot_a <= 0.0 || sq_norm < f32::EPSILON {
                    line_a
                } else {
                    let t = dot_a / sq_norm;
                    crate::coordinates::MapPoint::new(
                        line_a.x + t * line_vec.x,
                        line_a.y + t * line_vec.y,
                    )
                }
            }
        };

        // Reachability gate.  Note: `get_move_box(posture)` ignores
        // the posture argument and returns the current move box
        // unconditionally, so passing the current move box here is
        // the correct source even though the conceptual lookup is
        // for `posture_after_transition`.
        if !self.world.fast_grid.is_reachable_thick(
            pt_source.to_geo().into(),
            pt_new_goal.to_geo().into(),
            layer,
            half_diagonal,
        ) {
            return;
        }

        // Rewrite the last order's destination to the new goal.
        {
            let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            else {
                return;
            };
            let Some(last) = elem.orders.back_mut() else {
                return;
            };
            last.target_x = pt_new_goal.x;
            last.target_y = pt_new_goal.y;
        }

        // Backward collapse.  Starting at `num_orders - 3`, walk back
        // to `num_transition_orders`; for each source order that is
        // directly reachable from the new goal, delete the subsequent
        // order.  The initial snapshot of `num_orders` is
        // authoritative for the starting index because only the
        // intermediate orders between the loop cursor and the final
        // order are deleted.
        let n_orders_start = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.orders.len())
            .unwrap_or(0);
        if n_orders_start < 3 {
            return;
        }
        let mut i: i64 = n_orders_start as i64 - 3;
        while i >= num_transition_orders as i64 {
            let src_pt = {
                let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
                    return;
                };
                let Some(order) = elem.orders.get(i as usize) else {
                    return;
                };
                crate::coordinates::MapPoint::new(order.target_x, order.target_y)
            };
            // If an order somehow carries (0, 0) we log and bail
            // rather than silently producing bogus collapse geometry.
            if src_pt.x == 0.0 && src_pt.y == 0.0 {
                tracing::warn!(
                    ?seq_id,
                    elem_idx,
                    order_index = i,
                    "post_process_path_to_line: order has zero destination; aborting backward collapse"
                );
                break;
            }
            if self.world.fast_grid.is_reachable_thick(
                src_pt.to_geo().into(),
                pt_new_goal.to_geo().into(),
                layer,
                half_diagonal,
            ) {
                // Delete the order at i+1.
                {
                    let Some(elem) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(seq_id, elem_idx)
                    else {
                        return;
                    };
                    elem.orders.remove((i + 1) as usize);
                }
                let remaining = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|e| e.orders.len())
                    .unwrap_or(0);
                if remaining.saturating_sub(num_transition_orders) <= 2 {
                    break;
                }
            } else {
                break;
            }
            i -= 1;
        }
    }

    /// Return the actor's exact selected element (`mpSequenceElement`).
    fn selected_element(&self, entity: EntityId) -> Option<(SequenceId, usize)> {
        self.orders
            .sequence_manager
            .current_element_for_actor(entity)
    }

    /// Return the selected element only when that element itself is movement.
    /// Following or unrelated Todo movements do not own Actor::Make*'s
    /// pathfinder/PostProcessPath tail.
    fn selected_movement_element(&self, entity: EntityId) -> Option<(SequenceId, usize)> {
        let selected = self.selected_element(entity)?;
        self.orders
            .sequence_manager
            .get_element(selected.0, selected.1)
            .is_some_and(|element| element.data.is_movement())
            .then_some(selected)
    }

    /// Return the actor's pathfinder index only while its selected movement
    /// is waiting for path delivery. Original's posture-specific accessor is
    /// currently compiled to return this same primary index for every posture.
    fn pending_pathfinder_index(
        &self,
        entity: EntityId,
        (seq_id, elem_idx): (SequenceId, usize),
    ) -> Option<u16> {
        (self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|element| element.command)
            == Some(Command::MoveWaiting))
        .then(|| {
            self.get_entity(entity)
                .unwrap_or_else(|| panic!("pending movement owner {entity:?} is missing"))
                .position_iface()
                .get_pathfinder_index()
        })
    }

    /// Cumulative pixel distance of a given animation on the actor's
    /// current sprite. Returns `None` if the actor has no sprite or
    /// the animation isn't mapped to a row.
    fn sprite_distance_for_animation(&self, entity_id: EntityId, anim: OrderType) -> Option<i16> {
        let entity = self.get_entity(entity_id)?;
        let sprite = &entity.element_data().sprite;
        let distance = sprite.distance_for_animation(anim);
        tracing::trace!(
            owner = ?entity_id,
            ?anim,
            row = ?sprite.row_for_action(anim),
            distance,
            "transition animation distance lookup"
        );
        Some(distance)
    }
}

/// Decide which start-posture / start-action-state / end-transition
/// animations are required, based on the movement's action,
/// the owner's posture, the owner's action state, and whether the
/// next element in the sequence is itself a movement (or jump).
fn decide_transitions(
    animation_movement: OrderType,
    current_posture: Posture,
    current_action_state: ActionState,
    flags: MoveFlags,
    is_next_movement_or_jump: bool,
) -> (Option<OrderType>, Option<OrderType>, Option<OrderType>) {
    let mut animation_start_posture: Option<OrderType> = None;
    let mut animation_start_action_state: Option<OrderType> = None;
    let mut animation_end: Option<OrderType> = None;

    if matches!(current_posture, Posture::OnWall | Posture::OnLadder)
        && matches!(
            animation_movement,
            OrderType::WalkingUpright | OrderType::RunningUpright
        )
    {
        return (None, None, None);
    }

    match animation_movement {
        OrderType::WalkingUpright => {
            // Posture transition
            match current_posture {
                Posture::Upright => {}
                Posture::Crouched => {
                    animation_start_posture =
                        Some(if current_action_state == ActionState::Moving {
                            OrderType::TransitionWalkingCrouchedWalkingUpright
                        } else {
                            OrderType::TransitionCrouchingUp
                        });
                }
                Posture::Sitting => {
                    animation_start_posture = Some(OrderType::TransitionSittingWaitingUpright);
                }
                // Postures that don't need a start-posture transition
                // for walking.
                _ => {}
            }
            // Action-state transition
            match current_action_state {
                ActionState::Moving => {}
                ActionState::MovingFast => {
                    animation_start_action_state =
                        Some(OrderType::TransitionRunningUprightWalkingUpright);
                }
                ActionState::Waiting | ActionState::Bored => {
                    animation_start_action_state =
                        Some(OrderType::TransitionWaitingUprightWalkingUpright);
                }
                _ => {}
            }
        }
        OrderType::RunningUpright => {
            match current_posture {
                Posture::Upright
                | Posture::HelpingToClimb
                | Posture::SimulatingBeggar
                | Posture::OnShoulders => {}
                Posture::Crouched => {
                    animation_start_posture =
                        Some(OrderType::TransitionWalkingCrouchedRunningUpright);
                }
                Posture::Sitting => {
                    animation_start_posture = Some(OrderType::TransitionSittingWaitingUpright);
                }
                Posture::LeaningOut => {
                    animation_start_posture = Some(OrderType::TransitionLeaningOutWaitingAlerted);
                }
                _ => {}
            }
            match current_action_state {
                ActionState::MovingFast => {}
                ActionState::Moving if current_posture != Posture::Crouched => {
                    animation_start_action_state =
                        Some(OrderType::TransitionWalkingUprightRunningUpright);
                }
                ActionState::Waiting | ActionState::Bored
                    if current_posture != Posture::Crouched =>
                {
                    animation_start_action_state =
                        Some(OrderType::TransitionWaitingUprightRunningUpright);
                }
                s if s.is_shield() => {
                    // Any shield action state → LoweringShield. A
                    // shield-carrying actor that transitions to
                    // running plays the lowering-shield animation as
                    // the start-action-state transition.
                    animation_start_action_state = Some(OrderType::LoweringShield);
                }
                _ => {}
            }
        }
        OrderType::WalkingCrouched => {
            match current_posture {
                Posture::Upright => {
                    animation_start_posture = Some(match current_action_state {
                        ActionState::Moving => OrderType::TransitionWalkingUprightWalkingCrouched,
                        ActionState::MovingFast => {
                            OrderType::TransitionRunningUprightWalkingCrouched
                        }
                        _ => OrderType::TransitionCrouchingDown,
                    });
                }
                Posture::Crouched => {}
                _ => {}
            }
            match current_action_state {
                ActionState::MovingFast | ActionState::Moving => {}
                ActionState::Waiting | ActionState::Bored => {
                    animation_start_action_state =
                        Some(OrderType::TransitionWaitingCrouchedWalkingCrouched);
                }
                _ => {}
            }
        }
        // Animations that don't need any start/end transition logic.
        // Kept as an exhaustive allow list so the default arm doesn't
        // bite on new animations we haven't audited.
        OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast
        | OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast
        | OrderType::WalkingStairs
        | OrderType::RunningStairs
        | OrderType::WalkingWithSword
        | OrderType::RunningWithSword
        | OrderType::WalkingWithShield
        | OrderType::RiderCharging => {}
        _ => {}
    }

    if !is_next_movement_or_jump {
        animation_end = match animation_movement {
            OrderType::WalkingUpright => Some(OrderType::TransitionWalkingUprightWaitingUpright),
            OrderType::RunningUpright => Some(if flags.contains(MoveFlags::CHARGE) {
                OrderType::TransitionCharging
            } else {
                OrderType::TransitionRunningUprightWaitingUpright
            }),
            OrderType::WalkingCrouched => Some(OrderType::TransitionWalkingCrouchedWaitingCrouched),
            _ => None,
        };
    }

    (
        animation_start_posture,
        animation_start_action_state,
        animation_end,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActiveDoorPass, ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData,
        InstalledActorOrder, PcData,
    };
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    fn selected_running_pc() -> (EngineInner, EntityId, SequenceId, std::num::NonZeroU32) {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::RunningUpright,
        );
        movement
            .orders
            .push_back(Order::new(OrderType::RunningUpright, 10.0, 20.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        (engine, owner, sequence, order_id)
    }

    #[test]
    fn in_place_make_rewrite_updates_matching_installed_order_and_door_mirror() {
        let (mut engine, owner, sequence, order_id) = selected_running_pc();
        {
            let actor = engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.installed_order = Some(InstalledActorOrder {
                order_id,
                order_type: OrderType::RunningUpright,
            });
            actor.active_door_pass = Some(ActiveDoorPass {
                door_index: crate::gate::DoorIndex(0),
                direct: false,
                position_direct: false,
                steps: Default::default(),
                triggers_fired: 1,
                current_action: OrderType::RunningUpright,
                current_reverse: false,
                saved_action_state: None,
            });
        }
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .expect("selected movement order")
            .orders
            .front_mut()
            .expect("movement has an order")
            .order_type = OrderType::TransitionRunningUprightWalkingCrouched;

        engine.synchronize_rewritten_selected_order(owner, Some(OrderType::RunningUpright));

        let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
        assert_eq!(
            actor.installed_order,
            Some(InstalledActorOrder {
                order_id,
                order_type: OrderType::TransitionRunningUprightWalkingCrouched,
            })
        );
        assert_eq!(
            actor.active_door_pass.as_ref().unwrap().current_action,
            OrderType::TransitionRunningUprightWalkingCrouched
        );
    }

    #[test]
    fn make_crouched_publishes_rewritten_walk_before_inserting_posture_transition() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state: ActionState::Moving,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let walk_order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(owner),
            OrderType::WalkingUpright,
        );
        movement.orders.push_back(Order::new(
            OrderType::WalkingUpright,
            10.0,
            20.0,
            walk_order_id,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(owner)
            .expect("test PC")
            .actor_data_mut()
            .expect("test actor")
            .installed_order = Some(InstalledActorOrder {
            order_id: walk_order_id,
            order_type: OrderType::WalkingUpright,
        });

        engine.actor_make_crouched(&crate::sim_rng::test_context(), owner);

        let selected = engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .expect("post-processed movement remains selected")
            .2;
        assert_eq!(
            selected.order_type,
            OrderType::TransitionWalkingUprightWalkingCrouched,
            "PostProcessPath inserts the crouch transition after the rewritten walk"
        );
        assert_ne!(selected.order_id, walk_order_id);
        assert_eq!(
            engine
                .get_entity(owner)
                .expect("test PC")
                .actor_data()
                .expect("test actor")
                .installed_order,
            Some(InstalledActorOrder {
                order_id: walk_order_id,
                order_type: OrderType::WalkingCrouched,
            }),
            "the live installed order observes walk 6 -> crouched walk 16 before transition 81 is inserted"
        );
    }

    #[test]
    fn crouched_door_pass_make_fast_uses_crouched_running_transition_in_lazy_tail() {
        let (mut engine, owner, sequence, _) = selected_running_pc();
        {
            let entity = engine.get_entity_mut(owner).expect("test PC");
            entity.element_data_mut().posture = Posture::Crouched;
            let actor = entity.actor_data_mut().expect("test actor");
            actor.action_state = ActionState::Waiting;
            actor.active_door_pass = Some(ActiveDoorPass {
                door_index: crate::gate::DoorIndex(0),
                direct: false,
                position_direct: false,
                steps: VecDeque::from([DoorPassStep::Walk {
                    destination: MapPoint::new(30.0, 20.0),
                    action: OrderType::WalkingUpright,
                    reverse: false,
                    compute_direction: true,
                    tolerance: 0.0,
                }]),
                triggers_fired: 1,
                current_action: OrderType::TransitionCrouchingUp,
                current_reverse: false,
                saved_action_state: Some(ActionState::Waiting),
            });
        }
        // Model the materialized TransitionCrouchingUp prefix of the high
        // wall pass; the following walk remains in ActiveDoorPass::steps.
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .expect("selected movement")
            .orders
            .front_mut()
            .expect("materialized transition")
            .order_type = OrderType::TransitionCrouchingUp;

        engine.make_active_door_pass_fast(owner);

        let pass = engine
            .get_entity(owner)
            .expect("test PC")
            .actor_data()
            .expect("test actor")
            .active_door_pass
            .as_ref()
            .expect("active door pass");
        assert!(matches!(
            pass.steps.front(),
            Some(DoorPassStep::Walk {
                action: OrderType::TransitionWalkingCrouchedRunningUpright,
                ..
            })
        ));
    }

    #[test]
    fn sword_door_pass_make_fast_rewrites_lazy_tail_without_inserting_transition() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Crouched,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        }));
        let order_id = engine.orders.allocate_order_id();
        let mut movement = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingWithSword,
        );
        movement
            .orders
            .push_back(Order::new(OrderType::PassingDoor, 0.0, 0.0, order_id));
        let sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        {
            let actor = engine
                .get_entity_mut(owner)
                .expect("test PC")
                .actor_data_mut()
                .expect("test actor");
            actor.action_state = ActionState::MovingSword;
            actor.active_door_pass = Some(ActiveDoorPass {
                door_index: crate::gate::DoorIndex(0),
                direct: false,
                position_direct: false,
                steps: VecDeque::from([DoorPassStep::Walk {
                    destination: MapPoint::new(30.0, 20.0),
                    action: OrderType::WalkingUpright,
                    reverse: false,
                    compute_direction: true,
                    tolerance: 0.0,
                }]),
                triggers_fired: 1,
                current_action: OrderType::PassingDoor,
                current_reverse: false,
                saved_action_state: Some(ActionState::MovingSword),
            });
        }

        engine.actor_make_fast(&crate::sim_rng::test_context(), owner);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("selected sword door movement");
        let SequenceElementData::Movement { action, flags, .. } = &element.data else {
            panic!("movement element");
        };
        assert_eq!(*action, OrderType::RunningWithSword);
        assert!(flags.contains(MoveFlags::FAST));

        let pass = engine
            .get_entity(owner)
            .expect("test PC")
            .actor_data()
            .expect("test actor")
            .active_door_pass
            .as_ref()
            .expect("active door pass");
        assert_eq!(pass.steps.len(), 1, "sword speed changes add no transition");
        assert!(matches!(
            pass.steps.front(),
            Some(DoorPassStep::Walk {
                action: OrderType::RunningUpright,
                ..
            })
        ));
    }

    #[test]
    fn make_rewrite_does_not_install_a_different_selected_order_identity() {
        let (mut engine, owner, sequence, _) = selected_running_pc();
        let detached_id = engine.orders.allocate_order_id();
        engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .installed_order = Some(InstalledActorOrder {
            order_id: detached_id,
            order_type: OrderType::RunningUpright,
        });
        engine
            .orders
            .sequence_manager
            .get_element_mut(sequence, 0)
            .expect("selected movement order")
            .orders
            .front_mut()
            .expect("movement has an order")
            .order_type = OrderType::TransitionRunningUprightWalkingCrouched;

        engine.synchronize_rewritten_selected_order(owner, Some(OrderType::RunningUpright));

        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .installed_order,
            Some(InstalledActorOrder {
                order_id: detached_id,
                order_type: OrderType::RunningUpright,
            })
        );
    }
}
