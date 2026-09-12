//! Door step materialization and continuation ownership.
use super::super::movement::order_uses_distance_motion;
use super::*;

/// Keep the split door-pass walk mirror aligned with the concrete order that
/// has reached the actor slot.
///
/// Original stores the translated door route and posture-transition copies in
/// one order list. Once a transition retires, the following walk/run order is
/// immediately authoritative. Rust keeps the untranslated tail in
/// `ActiveDoorPass`; without this rebind a transition written into
/// `current_action` by speed changes can continue supplying the sprite row
/// while the concrete successor is already executing.
pub(in crate::engine) fn synchronize_selected_door_pass_walk_action(
    current_action: &mut OrderType,
    selected_action: OrderType,
) {
    if order_uses_distance_motion(selected_action) {
        *current_action = selected_action;
    }
}

/// Delete every following order when a transition loops short of its goal and
/// cannot find a later
/// distinct animation with a nonzero destination.
///
/// The translated door route normally lives in that one order list. This
/// implementation stores its untranslated suffix separately, so the same
/// deletion must also empty `ActiveDoorPass::steps` or a discarded
/// zero-destination `PASSING_DOOR` action point will be materialized again.
pub(in crate::engine) fn discard_lazy_door_pass_following_orders(
    pass: Option<&mut ActiveDoorPass>,
) {
    if let Some(pass) = pass {
        pass.clear_pending_steps();
    }
}

/// Materialize the zero-destination action points which precede the next
/// authored door walk.
///
/// The original game translates the complete door route up front. Rust normally
/// materializes these steps one at a time, but a TillLastFrame continuation
/// has to be inserted relative to the complete translated route. Moving this
/// prefix into the concrete order queue first preserves Original's
/// `Select -> PassingDoor -> copied walk` ordering.
pub(in crate::engine) fn materialize_door_action_point_prefix(
    pass: &mut ActiveDoorPass,
    next_order_id: &mut u32,
) -> Vec<crate::order::Order> {
    pass.align_pending_order_ids();
    let mut orders = Vec::new();
    while matches!(
        pass.steps.front(),
        Some(
            crate::element::DoorPassStep::Select { .. } | crate::element::DoorPassStep::PassingDoor
        )
    ) {
        let (step, reserved_id) = pass
            .pop_pending_step()
            .expect("inspected door action point disappeared");
        let order_id = reserved_id.unwrap_or_else(|| crate::order::alloc_order_id(next_order_id));
        let mut order = match step {
            crate::element::DoorPassStep::Select { speed } => {
                let mut order = crate::order::Order::new(OrderType::Select, 0.0, 0.0, order_id);
                order.compute_direction = true;
                order.tolerance = speed;
                order
            }
            crate::element::DoorPassStep::PassingDoor => {
                crate::order::Order::new(OrderType::PassingDoor, 0.0, 0.0, order_id)
            }
            _ => unreachable!("only inspected door action points are consumed"),
        };
        // These action points now have concrete successors in the same order
        // list, so generic order advancement owns their completion. Door-pass resumption
        // would incorrectly materialize another lazy step alongside them.
        order.completion = crate::order::OrderCompletion::AdvanceElement;
        orders.push(order);
    }
    orders
}

/// Insert a materialized lazy door-pass step at the same side of a copied
/// transition-distance continuation as Original's single translated order
/// list.
///
/// Motion through the last frame inserts the copied continuation immediately
/// before the first later, distinct animation with a nonzero destination. Rust
/// stores the orders preceding that animation (PassingDoor and zero-target
/// posture transitions) in `ActiveDoorPass`, so those steps must be inserted
/// before the concrete continuation. The matching authored walk belongs after
/// it.
pub(in crate::engine) fn insert_door_pass_successor(
    element: &mut crate::sequence::SequenceElement,
    order: crate::order::Order,
) {
    let continuation = element
        .orders
        .iter()
        .position(|queued| queued.transition_distance_continuation);
    let Some(continuation) = continuation else {
        element.push_order(order);
        return;
    };
    let copied = &element.orders[continuation];
    let is_matching_authored_walk =
        (order.target_x != 0.0 || order.target_y != 0.0) && order.order_type == copied.order_type;
    let insertion = continuation + usize::from(is_matching_authored_walk);
    element.insert_order(insertion, order);
}

pub(in crate::engine) fn completed_door_pass_to_commit(
    discarded_following_orders: bool,
    completed: Option<(crate::gate::DoorIndex, bool)>,
) -> Option<(crate::gate::DoorIndex, bool)> {
    (!discarded_following_orders).then_some(completed).flatten()
}

/// Ignore a stale split door-route mirror when a distinct concrete transition
/// has reached the actor slot.
///
/// Original keeps the whole translated route in one order list, so an
/// explicit transition inserted by speed changes is authoritative. Rust's
/// mirror remains useful for concrete distance motion and for door-authored
/// transitions where it already agrees with the selected order.
pub(in crate::engine) fn door_pass_sprite_animation_override(
    selected_action: OrderType,
    current_action: Option<OrderType>,
) -> Option<OrderType> {
    current_action.filter(|current| {
        order_uses_distance_motion(selected_action) || *current == selected_action
    })
}

/// Posture owned eagerly by a lift animation while executing a door-pass
/// step. Wall-exit transitions are different from the climb rows: they only
/// inherit `OnWall` when the transition is
/// initialized, then its raw `DONE` edge is allowed to publish the landing
/// posture while the animation wrapper remains installed.
pub(in crate::engine) fn door_pass_eager_posture(
    action: OrderType,
    has_door_pass_animation: bool,
    execute_order_initialising: bool,
    decorative_building_trap_at_destination: bool,
) -> Option<crate::element::Posture> {
    use crate::element::Posture;

    if !has_door_pass_animation || decorative_building_trap_at_destination {
        return None;
    }
    match action {
        OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast => Some(Posture::OnWall),
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            if execute_order_initialising =>
        {
            Some(Posture::Flying)
        }
        OrderType::TransitionClimbingWallUpWaitingCrouched
        | OrderType::TransitionClimbingWallDownWaitingUpright
            if execute_order_initialising =>
        {
            Some(Posture::OnWall)
        }
        OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast => Some(Posture::OnLadder),
        _ => None,
    }
}

/// Result of [`EngineInner::advance_door_pass`].
///
/// Outcomes from draining the order list after a walk step terminates.
#[derive(Debug, Clone)]
pub(in crate::engine) enum DoorPassAdvance {
    /// No active door pass existed when the state machine was asked to
    /// advance. This is a caller bug or a stale animation callback; it
    /// must not be treated as a completed pass.
    NoActive,
    /// A new `Walk` step is ready — the caller must push a walking
    /// order onto the actor's current sequence element to install the
    /// destination.  Movement tick resumes once the order is queued.
    Continue {
        order_id: std::num::NonZeroU32,
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
    /// A non-animation `PassingDoor` action point is ready. It must be
    /// installed as the next real actor order so it consumes its own owner
    /// slot, just like the Original order chain.
    ActionPoint { order: crate::order::Order },
    /// No more steps remain; the door pass is complete and the caller
    /// should tear down path / active-movement state.
    Done {
        completed: Option<(crate::gate::DoorIndex, bool)>,
    },
}

pub(in crate::engine) fn clear_terminal_door_pass_goal(entity: &mut Entity) {
    entity
        .position_iface_mut()
        .set_map_goal(crate::coordinates::MapPoint::ZERO);
}

impl EngineInner {
    /// Advance through door-pass steps after a walk step completes.
    ///
    /// Pops one translated motion/door sub-order. `PassingDoor` action
    /// points are returned as real orders instead of being drained in the
    /// predecessor's completion slot: the original-game frame update executes one
    /// current order and only then advances to its successor. `Select` likewise
    /// returns a real order; its own Execute slot owns the hulk callback.
    ///
    /// See [`DoorPassAdvance`] for return semantics.
    pub(in crate::engine) fn advance_door_pass(
        actor: &mut crate::element::ActorData,
        entity_id: EntityId,
        transition_destination: MapPoint,
        next_order_id: &mut u32,
    ) -> DoorPassAdvance {
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
        let (step, preallocated_order_id) = match dp.pop_pending_step() {
            Some(pending) => pending,
            None => {
                let completed = Some((dp.door_index, dp.direct));
                actor.active_door_pass = None;
                return DoorPassAdvance::Done { completed };
            }
        };
        let mut order_id =
            || preallocated_order_id.unwrap_or_else(|| crate::order::alloc_order_id(next_order_id));

        match step {
            crate::element::DoorPassStep::PassingDoor => {
                let order = crate::order::Order::new(OrderType::PassingDoor, 0.0, 0.0, order_id());
                DoorPassAdvance::ActionPoint { order }
            }
            crate::element::DoorPassStep::Select { speed } => {
                // The original game translates selection into a real non-animation order:
                // it is promoted after the preceding walk, executes the Human
                // hulk side effect in its own actor slot, then resumes the
                // remaining door chain. Skipping it advances PASSING_DOOR and
                // its topology swap by one frame.
                let mut order = crate::order::Order::new(OrderType::Select, 0.0, 0.0, order_id());
                order.compute_direction = true;
                order.tolerance = speed;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                DoorPassAdvance::ActionPoint { order }
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
                // Save the walking action state for the post-transition
                // walk.  Merely materializing the successor must not change
                // it yet: Original does not execute the transition until its
                // own update slot starts on the following tick.
                let saved = actor.action_state;
                actor.clear_path();
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.saved_action_state = Some(saved);
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                let mut order = crate::order::Order::new(
                    action,
                    transition_destination.x,
                    transition_destination.y,
                    order_id(),
                );
                order.reverse = reverse;
                order.compute_direction = false;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                tracing::debug!(
                    entity = ?entity_id,
                    ?action,
                    reverse,
                    "DoorPass: pausing for Transition animation"
                );
                DoorPassAdvance::Paused {
                    transition_order: order,
                }
            }
            crate::element::DoorPassStep::Walk {
                destination,
                action,
                reverse,
                compute_direction,
                tolerance,
            } => {
                // The walk animation itself comes from `current_action`
                // (read by tick_entity_movement via `door_pass_anim`).  Keep
                // the saved pre-transition state until this new order is
                // actually dispatched on the following owner tick.
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                // Hand the Walk destination back to the caller —
                // advance_door_pass doesn't have sequence_manager
                // access, so it can't push the walking order
                // directly onto the PassDoor element.  The caller
                // (tick_entity_movement's post-loop door-pass
                // dispatch) does the order push.
                DoorPassAdvance::Continue {
                    order_id: order_id(),
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                }
            }
        }
    }
}
