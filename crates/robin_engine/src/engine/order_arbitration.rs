//! Instruct arbitration, postponement, and owner stop boundaries.
//!
//! Cross-system callback orchestration stays on `EngineInner`; mechanics
//! cleanup and order-state updates borrow only their world/order domains.
use super::state::OrderRuntime;
use super::*;

impl EngineInner {
    /// Apply the human actor's specialized admission guard. It runs before
    /// base actor instruction, so a rejected
    /// command must not stamp transition state or allocate transition orders.
    pub(in crate::engine) fn human_instruct_rejects_command(
        &self,
        owner: EntityId,
        command: crate::element::Command,
    ) -> bool {
        let Some(entity) = self.get_entity(owner) else {
            return false;
        };
        if !entity.is_human() {
            return false;
        }

        let is_dead = entity.is_dead();
        let is_unconscious = entity.human_data().is_some_and(|human| human.unconscious);
        let stuck_counter = entity
            .human_data()
            .map(|human| human.stuck_under_nets_counter)
            .unwrap_or(0);
        if !is_dead && !is_unconscious && stuck_counter == 0 {
            return false;
        }

        let allowed = matches!(
            command,
            crate::element::Command::ReceiveHitDamage
                | crate::element::Command::ReceiveSwordDamage
                | crate::element::Command::ReceiveArrowDamage
                | crate::element::Command::ReceiveDamage
                | crate::element::Command::ReceiveMobileDamage
                | crate::element::Command::Wait
                | crate::element::Command::GetKilledAtBottom
        ) || (command == crate::element::Command::ReceiveNet
            && !is_dead
            && !is_unconscious
            && stuck_counter == 1);
        !allowed
    }

    /// Compare the incoming instruction with the selected element and finish
    /// any replacement callbacks before translation. Returns whether the
    /// incoming element remains accepted after those callbacks.
    pub(crate) fn arbitrate_instruct(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        new_seq: crate::sequence::SequenceId,
        new_idx: usize,
    ) -> bool {
        use crate::sequence::{PriorityDecision, SequenceState};

        let Some(new_elem) = self.orders.sequence_manager.get_element(new_seq, new_idx) else {
            return false;
        };
        let Some(owner) = new_elem.owner else {
            // No owner: nothing to arbitrate against, let it through.
            return true;
        };
        self.trace_attentive_owner_handoff(
            "instruct_entry",
            owner,
            Some((new_seq, new_idx)),
            format_args!("before admission and priority arbitration"),
        );
        // Idempotency guard.  Owned launches now arbitrate
        // synchronously inside `launch_element_for_owner`, but legacy
        // callsites that explicitly arbitrate after an owned launch still hit
        // `arbitrate_instruct` explicitly after `launch_element`.  The
        // second call must be a safe no-op: if the first call already
        // resolved the element, return the matching bool without
        // repeating the decision (which would double-postpone / double-
        // interrupt on cascading priorities).
        match new_elem.state {
            // Re-registering postponed work preserves its state until Go
            // instructs the owner. It must pass the same admission as new work.
            SequenceState::Todo | SequenceState::Postponed => {}
            SequenceState::InProgress => {
                // The element is already selected. Accept it without comparing the
                // element against itself: a postponed element can retain its
                // original manager registration and gain a second one when
                // its blocker releases it, so duplicate InstructOwner
                // actions are possible in the same drain.
                if self.current_sequence_element_for_actor(owner) == Some((new_seq, new_idx)) {
                    return true;
                }
            }
            SequenceState::Impossible
            | SequenceState::Interrupted
            | SequenceState::Terminated
            | SequenceState::Done => {
                tracing::trace!(
                    ?owner,
                    ?new_seq,
                    new_idx,
                    command = ?new_elem.command,
                    state = ?new_elem.state,
                    "arbitrate_instruct skipped a non-pending element"
                );
                return false;
            }
        }
        let new_priority = new_elem.priority;
        let new_command = new_elem.command;

        // Every recipient of an instruction is unconditionally unfrozen
        // before the arbitration / dispatch logic runs.  Without this
        // clear, a freeze imposed via paths other than `DropDone`
        // (which clears it synchronously) would persist past the next instruction.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.execution_frozen = false;
        }

        // The posture / action-state stamp now runs at *launch* time
        // via `launch_element_for_owner` and the stamped
        // single-order-sequence wrapper, synchronous with the
        // launch → instruction flow. By the time arbitration runs, the
        // stamp is already on the element.

        // ── Actor-specific instruction rules ─────────────────────────
        //
        // Civilian instruction handling refuses everything except RECEIVE_PURSE /
        // BEGGAR_SHOW_FACE / WAIT when the civilian is a beggar.
        if self.beggar_rejects_command(owner, new_command) {
            self.element_impossible(sim, assets, active_scripts, new_seq, new_idx);
            return false;
        }

        if self.human_instruct_rejects_command(owner, new_command) {
            self.element_impossible(sim, assets, active_scripts, new_seq, new_idx);
            return false;
        }

        let Some((cur_seq, cur_idx)) = self.current_sequence_element_for_actor(owner) else {
            // Idle actor — new element takes over unconditionally.
            self.select_sequence_element(owner, Some((new_seq, new_idx)));
            return true;
        };

        let cur_priority = self
            .orders
            .sequence_manager
            .get_element(cur_seq, cur_idx)
            .map(|e| e.priority)
            .unwrap_or(crate::sequence::SequencePriority::None);

        let decision = crate::sequence::decide_priorities(cur_priority, new_priority);

        self.trace_attentive_owner_handoff(
            "instruct_decision",
            owner,
            Some((new_seq, new_idx)),
            format_args!(
                "current={cur_seq:?}/{cur_idx} current_priority={cur_priority:?} incoming_command={new_command:?} incoming_priority={new_priority:?} decision={decision:?}"
            ),
        );

        tracing::trace!(
            ?owner,
            ?cur_seq,
            cur_idx,
            ?cur_priority,
            ?new_seq,
            new_idx,
            ?new_priority,
            ?decision,
            "arbitrate_instruct"
        );

        match decision {
            PriorityDecision::Abandon => {
                // Hand the new element's postponed successor (if any)
                // over to the current element before marking new
                // Impossible, so the successor doesn't get orphaned.
                self.orders
                    .sequence_manager
                    .take_over_postponed(cur_seq, cur_idx, new_seq, new_idx);
                self.element_impossible(sim, assets, active_scripts, new_seq, new_idx);
                false
            }
            PriorityDecision::Postpone => {
                // `new.Postpone(current)` — may recurse when the target
                // already has a postponed chain.
                self.engine_postpone(
                    sim,
                    assets,
                    active_scripts,
                    cur_seq,
                    cur_idx,
                    new_seq,
                    new_idx,
                );
                false
            }
            PriorityDecision::PostponeCurrent => {
                assert!(
                    self.orders
                        .sequence_manager
                        .can_interrupt_now(cur_seq, cur_idx),
                    "interruption eligibility is unconditional"
                );
                // `current.Postpone(new)` — postpone current behind new.
                // Current is in-progress, so we first tear down its
                // active machinery before flipping it to Postponed.
                // human action execution's WAITING_SWORD branch and base actor execution's
                // bored upright-waiting arms always return an in-progress result
                // after driving their nested work. If that work
                // synchronously installs an injury while WAIT_FREE_LIFT owns
                // one of those orders, a transient sprite-cycle Done must not
                // trigger engine_postpone's usual done-order shortcut. The
                // blocked lift waiter itself is what Original postpones.
                // Keep the exception at this exact arbitration seam so every
                // other done-order race, including WAIT_TIMER, is unchanged.
                let preserve_nonterminating_lift_wait = self
                    .orders
                    .sequence_manager
                    .get_element(cur_seq, cur_idx)
                    .is_some_and(|element| {
                        element.command == crate::element::Command::WaitFreeLift
                            && element.state == crate::sequence::SequenceState::InProgress
                            && element.orders.back().is_some_and(|order| {
                                matches!(
                                    order.order_type,
                                    crate::order::OrderType::WaitingSword
                                        | crate::order::OrderType::WaitingUprightBored
                                        | crate::order::OrderType::WaitingUprightBoredRandom
                                ) && order.done
                            })
                    });
                stop_owner_active_mechanics(&mut self.orders, owner);
                if preserve_nonterminating_lift_wait
                    && let Some(order) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(cur_seq, cur_idx)
                        .and_then(|element| element.orders.back_mut())
                {
                    order.done = false;
                }
                // Callbacks observe the incoming instruction before the outgoing
                // element is postponed, and may replace it recursively.
                self.select_sequence_element(owner, Some((new_seq, new_idx)));
                self.engine_postpone(
                    sim,
                    assets,
                    active_scripts,
                    new_seq,
                    new_idx,
                    cur_seq,
                    cur_idx,
                );
                self.current_sequence_element_for_actor(owner) == Some((new_seq, new_idx))
            }
            PriorityDecision::InterruptCurrent => {
                assert!(
                    self.orders
                        .sequence_manager
                        .can_interrupt_now(cur_seq, cur_idx),
                    "interruption eligibility is unconditional"
                );
                // New takes over current's postponed chain, current
                // becomes Interrupted.
                self.orders
                    .sequence_manager
                    .take_over_postponed(new_seq, new_idx, cur_seq, cur_idx);
                stop_owner_active_mechanics(&mut self.orders, owner);
                // Select before interruption so nested callbacks arbitrate against
                // the incoming instruction. Returning never restores selection.
                self.select_sequence_element(owner, Some((new_seq, new_idx)));
                self.element_interrupted(
                    sim,
                    assets,
                    active_scripts,
                    cur_seq,
                    cur_idx,
                    crate::sequence::CascadeFlags::NEXT_LEVEL,
                );
                self.current_sequence_element_for_actor(owner) == Some((new_seq, new_idx))
            }
        }
    }

    /// Postpone element `waiter` behind element `blocker` on the same
    /// actor.  When the blocker already has a postponed successor,
    /// arbitrate between the existing successor and the new waiter —
    /// may recurse, swap, or interrupt deeper in the chain.
    pub(super) fn engine_postpone(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        blocker_seq: crate::sequence::SequenceId,
        blocker_idx: usize,
        waiter_seq: crate::sequence::SequenceId,
        waiter_idx: usize,
    ) {
        self.engine_postpone_with_debug_depth(
            sim,
            assets,
            active_scripts,
            blocker_seq,
            blocker_idx,
            waiter_seq,
            waiter_idx,
            0,
        );
    }

    pub(super) fn engine_postpone_with_debug_depth(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        blocker_seq: crate::sequence::SequenceId,
        blocker_idx: usize,
        waiter_seq: crate::sequence::SequenceId,
        waiter_idx: usize,
        depth: usize,
    ) {
        use crate::sequence::PriorityDecision;

        let mut blocker_seq = blocker_seq;
        let mut blocker_idx = blocker_idx;
        let mut depth = depth;
        let append_root = (blocker_seq, blocker_idx);
        let waiter_priority = self
            .orders
            .sequence_manager
            .get_element(waiter_seq, waiter_idx)
            .map(|element| element.priority)
            .unwrap_or_else(|| panic!("postpone waiter {waiter_seq:?}/{waiter_idx} is missing"));
        let (append_point, skipped_hops, cacheable_append) = self
            .orders
            .sequence_manager
            .postpone_append_point(append_root, waiter_priority);
        blocker_seq = append_point.0;
        blocker_idx = append_point.1;
        depth += skipped_hops;

        // A single actor can legitimately retain thousands of equal-priority
        // postponed elements. Original walks that chain recursively, but a
        // Rust frame for this dispatcher is substantially larger and can
        // exhaust the process stack first. The `Postpone` arm is a pure tail
        // call, so walk that arm iteratively while retaining recursion for the
        // non-tail `PostponeCurrent` topology rewrite.
        loop {
            assert_ne!(
                (blocker_seq, blocker_idx),
                (waiter_seq, waiter_idx),
                "engine_postpone cannot postpone a sequence element behind itself"
            );

            tracing::trace!(
                target: "parity_launch",
                depth,
                blocker = ?(blocker_seq, blocker_idx),
                waiter = ?(waiter_seq, waiter_idx),
                "engine_postpone enter"
            );

            if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
                let sequence_graph = |seq_id| {
                    self.orders
                        .sequence_manager
                        .get_sequence(seq_id)
                        .map(|sequence| {
                            sequence
                                .elements
                                .iter()
                                .enumerate()
                                .map(|(index, element)| {
                                    (
                                        index,
                                        element.owner,
                                        element.command,
                                        element.command_level,
                                        element.state,
                                        element.priority,
                                        element
                                            .orders
                                            .iter()
                                            .map(|order| {
                                                (order.order_type, order.order_id, order.done)
                                            })
                                            .collect::<Vec<_>>(),
                                        element.postponed.map(|reference| {
                                            (reference.sequence_id, reference.element_index)
                                        }),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                };
                let waiter_last_order = self
                    .orders
                    .sequence_manager
                    .get_element(waiter_seq, waiter_idx)
                    .and_then(|element| {
                        element
                            .orders
                            .back()
                            .map(|order| (order.order_type, order.order_id, order.done))
                    });
                let blocker_graph = sequence_graph(blocker_seq);
                let waiter_graph = sequence_graph(waiter_seq);
                tracing::trace!(
                    target: "parity_owner_handoff",
                    frame = self.control.frame_counter,
                    depth,
                    blocker = ?(blocker_seq, blocker_idx),
                    waiter = ?(waiter_seq, waiter_idx),
                    ?waiter_last_order,
                    ?blocker_graph,
                    ?waiter_graph,
                    "engine_postpone before topology arbitration"
                );
            }

            // If blocker already has a postponed successor, arbitrate
            // between that existing successor and the new waiter.
            let existing_postponed = self
                .orders
                .sequence_manager
                .get_element(blocker_seq, blocker_idx)
                .and_then(|e| {
                    e.postponed
                        .map(|reference| (reference.sequence_id, reference.element_index))
                });
            if let Some((existing_seq, existing_idx)) = existing_postponed {
                tracing::trace!(
                    target: "parity_launch",
                    depth,
                    blocker = ?(blocker_seq, blocker_idx),
                    existing = ?(existing_seq, existing_idx),
                    "engine_postpone existing"
                );
                let existing_priority = self
                    .orders
                    .sequence_manager
                    .get_element(existing_seq, existing_idx)
                    .map(|e| e.priority)
                    .unwrap_or(crate::sequence::SequencePriority::None);
                let waiter_priority = self
                    .orders
                    .sequence_manager
                    .get_element(waiter_seq, waiter_idx)
                    .map(|e| e.priority)
                    .unwrap_or(crate::sequence::SequencePriority::None);

                let decision =
                    crate::sequence::decide_priorities(existing_priority, waiter_priority);
                tracing::trace!(
                    target: "parity_owner_handoff",
                    frame = self.control.frame_counter,
                    depth,
                    blocker = ?(blocker_seq, blocker_idx),
                    existing = ?(existing_seq, existing_idx),
                    waiter = ?(waiter_seq, waiter_idx),
                    ?existing_priority,
                    ?waiter_priority,
                    ?decision,
                    "engine_postpone existing-successor branch"
                );
                match decision {
                    PriorityDecision::Abandon => {
                        // existing wins — take over waiter's postponed
                        // chain and abandon waiter.
                        self.orders.sequence_manager.take_over_postponed(
                            existing_seq,
                            existing_idx,
                            waiter_seq,
                            waiter_idx,
                        );
                        self.element_impossible(
                            sim,
                            assets,
                            active_scripts,
                            waiter_seq,
                            waiter_idx,
                        );
                        return;
                    }
                    PriorityDecision::Postpone => {
                        // Waiter queues behind existing. This is a tail call;
                        // continue iteratively so a long legitimate postponed
                        // chain cannot overflow Rust's larger dispatcher stack.
                        blocker_seq = existing_seq;
                        blocker_idx = existing_idx;
                        depth += 1;
                        continue;
                    }
                    PriorityDecision::PostponeCurrent => {
                        // existing becomes postponed behind waiter.  Keep
                        // blocker→waiter link (set below after the fall-
                        // through) and install existing behind waiter.
                        // First detach existing from blocker's slot so we
                        // don't leave a dangling link while recursing.
                        self.orders
                            .sequence_manager
                            .set_cross_postponed_link((blocker_seq, blocker_idx), None);
                        self.engine_postpone_with_debug_depth(
                            sim,
                            assets,
                            active_scripts,
                            waiter_seq,
                            waiter_idx,
                            existing_seq,
                            existing_idx,
                            depth + 1,
                        );
                        // Fall through to install waiter in blocker's slot.
                    }
                    PriorityDecision::InterruptCurrent => {
                        // waiter inherits existing's postponed chain;
                        // existing becomes interrupted. The original game's state change calls
                        // removal notification synchronously before the outer
                        // Instruction handling resumes and installs waiter in blocker's slot.
                        self.orders.sequence_manager.take_over_postponed(
                            waiter_seq,
                            waiter_idx,
                            existing_seq,
                            existing_idx,
                        );
                        self.orders
                            .sequence_manager
                            .set_cross_postponed_link((blocker_seq, blocker_idx), None);
                        self.prepare_cross_postponed_waiter(
                            sim,
                            assets,
                            active_scripts,
                            waiter_seq,
                            waiter_idx,
                        );
                        self.element_interrupted(
                            sim,
                            assets,
                            active_scripts,
                            existing_seq,
                            existing_idx,
                            crate::sequence::CascadeFlags::NEXT_LEVEL,
                        );
                        self.orders.sequence_manager.set_cross_postponed_link(
                            (blocker_seq, blocker_idx),
                            Some((waiter_seq, waiter_idx)),
                        );
                        return;
                    }
                }
            }

            // When the waiter already has orders and its last order is
            // done, just terminate it instead of postponing.  Otherwise
            // install it in the blocker's postponed slot.
            let should_terminate_instead = self
                .orders
                .sequence_manager
                .get_element(waiter_seq, waiter_idx)
                .map(|e| {
                    e.command != crate::element::Command::MoveOk
                        && e.orders.back().is_some_and(|o| o.done)
                })
                .unwrap_or(false);

            tracing::trace!(
                target: "parity_owner_handoff",
                frame = self.control.frame_counter,
                depth,
                blocker = ?(blocker_seq, blocker_idx),
                waiter = ?(waiter_seq, waiter_idx),
                should_terminate_instead,
                branch = if should_terminate_instead {
                    "terminate_done_waiter"
                } else {
                    "install_postponed_waiter"
                },
                "engine_postpone final branch"
            );

            if should_terminate_instead {
                if let Some(e) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(waiter_seq, waiter_idx)
                {
                    e.orders.clear();
                }
                self.element_terminated(sim, assets, active_scripts, waiter_seq, waiter_idx);
                return;
            }

            if cacheable_append {
                self.orders.sequence_manager.install_cached_postpone_append(
                    append_root,
                    waiter_priority,
                    (blocker_seq, blocker_idx),
                    (waiter_seq, waiter_idx),
                    skipped_hops,
                );
            } else {
                self.orders.sequence_manager.set_cross_postponed_link(
                    (blocker_seq, blocker_idx),
                    Some((waiter_seq, waiter_idx)),
                );
            }
            self.prepare_cross_postponed_waiter(
                sim,
                assets,
                active_scripts,
                waiter_seq,
                waiter_idx,
            );
            tracing::trace!(
                target: "parity_launch",
                depth,
                blocker = ?(blocker_seq, blocker_idx),
                waiter = ?(waiter_seq, waiter_idx),
                "engine_postpone exit"
            );
            return;
        }
    }

    /// Stop all active / pending sequence elements owned by `owner`,
    /// rewriting any in-progress movement element's current order to
    /// the matching waiting-transition animation (shortened to ~10
    /// units) and cancelling pending pathfinder requests.
    ///
    /// This is the full `Stop()` path — combining the actor stop, the
    /// sequence-manager not-yet-launched stop, the movement-element
    /// movement stopping, and conditional path cancellation. Callers that
    /// previously invoked `self.orders.sequence_manager.stop_owner(...)`
    /// directly should use this wrapper so the actor's movement doesn't
    /// keep running on a stale path.
    pub(crate) fn stop_actor_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: crate::sequence::SequencePriority,
    ) {
        self.stop_actor_orders_phase(sim, assets, active_scripts, owner, stop_priority, true);
    }

    /// Run the actor-selected half of actor stopping, leaving the
    /// not-yet-launched queue untouched until the caller has closed the
    /// selected element's synchronous condolence callback.
    pub(super) fn stop_owner_current(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: crate::sequence::SequencePriority,
    ) {
        self.stop_actor_orders_phase(sim, assets, active_scripts, owner, stop_priority, false);
    }

    /// Finish actor stopping after the selected element's synchronous
    /// condolence callback. Original snapshots the pending-list length, then
    /// each stopped entry sends its own card before the scan advances.
    pub(super) fn stop_owner_pending_after_callback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: crate::sequence::SequencePriority,
    ) {
        let resolver = |engine: &EngineInner, element: &crate::sequence::SequenceElement| {
            Self::priority_resolver(&engine.world.entities)(element)
        };
        self.stop_pending_elements(sim, assets, active_scripts, owner, stop_priority, &resolver);
    }
}

/// Walk every actor whose sprite reported `MotionState::Done` this
/// tick and flip `done = true` on the actor's currently-dispatched
/// order, then clear `last_motion_state` on every sprite so the
/// field is fresh for the next tick.
///
/// Sprite advancement is split across the live owner coordinator and
/// the remaining specialized arms (`tick_actor_animation_for`, active
/// jumps, melee, bow, and abilities); each one funnels through
/// [`Sprite::record_motion_state`](crate::sprite::Sprite), which
/// stashes the result in [`Sprite::last_motion_state`].  This pass
/// runs once per frame after every per-system tick has completed,
/// recovering the "single Done observer" semantics without forcing
/// each per-system tick to know about the order-completion flag.
///
/// The corresponding read site is the postpone-race guard in
/// [`EngineInner::engine_postpone`]: when a postpone target's last order
/// is already `done`, the postpone short-circuits to TERMINATED
/// instead of installing the cross-element link.
pub(super) fn propagate_done_to_current_orders(
    entities: &mut crate::entities::Entities,
    sequence_manager: &mut crate::sequence::SequenceManager,
) {
    let done_actors: Vec<(crate::element::EntityId, u32)> = entities
        .actors()
        .filter_map(|(entity_id, entity)| {
            matches!(
                entity.element_data().sprite.last_motion_state,
                Some(crate::sprite::MotionState::Done)
            )
            .then_some((
                entity_id.into(),
                entity.element_data().sprite.last_processed_order_id,
            ))
        })
        .collect();

    for (entity_id, processed_order_id) in done_actors {
        let Some((seq_id, elem_idx)) = entities.current_element_for_actor(entity_id) else {
            continue;
        };
        if let Some(elem) = sequence_manager.get_element_mut(seq_id, elem_idx)
            && let Some(order) = elem.orders.front_mut()
            && order.order_id.get() == processed_order_id
        {
            order.done = true;
        }
    }

    // Reset every sprite's transient last_motion_state so the next
    // tick starts clean, regardless of whether the slot was an
    // actor or had an order to mark.
    for (_, entity) in entities.occupied_mut() {
        entity.element_data_mut().sprite.last_motion_state = None;
    }
}

impl EngineInner {
    fn prepare_cross_postponed_waiter(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,

        waiter_seq: crate::sequence::SequenceId,
        waiter_idx: usize,
    ) {
        // Postponing a movement element restores a
        // translated movement element to its untranslated command before the
        // common sequence-state transition runs. A resumed element is sent
        // through instruction/translation again, so retaining MoveWaiting or MoveOk
        // here would either strand the old pathfinder state or bypass path
        // translation entirely.
        //
        // Preserve the original game's postponed-movement behavior.
        let postponed_movement = self
            .orders
            .sequence_manager
            .get_element(waiter_seq, waiter_idx)
            .and_then(|element| {
                matches!(
                    element.command,
                    crate::element::Command::MoveWaiting | crate::element::Command::MoveOk
                )
                .then_some((element.owner, element.command))
            });
        if let Some((owner, command)) = postponed_movement {
            if command == crate::element::Command::MoveWaiting {
                let owner = owner.unwrap_or_else(|| {
                panic!(
                    "MoveWaiting element {waiter_seq:?}[{waiter_idx}] has no actor owner while being postponed"
                )
            });
                self.orders.pending_path_requests.cancel_for_owner(owner);
                self.orders
                    .failed_path_requests
                    .retain(|request| request.owner != owner);
            }
            self.orders
                .sequence_manager
                .get_element_mut(waiter_seq, waiter_idx)
                .expect("postponed movement element disappeared")
                .command = crate::element::Command::Move;
        }

        if let Some(w) = self
            .orders
            .sequence_manager
            .get_element_mut(waiter_seq, waiter_idx)
        {
            w.orders.clear();
        }
        self.postpone_element(sim, assets, active_scripts, waiter_seq, waiter_idx);
    }
}
/// Cancel pending and failed path requests before interrupting or postponing
/// the actor's selected element.
pub(super) fn stop_owner_active_mechanics(orders: &mut OrderRuntime, owner: EntityId) {
    orders.pending_path_requests.cancel_for_owner(owner);
    // Path-request cancellation fires from both
    // interrupted *and* postponed state changes, and
    // drops stale retry entries for the actor.  Mirror that here so
    // cross-postpone (higher-priority blocker) also evicts pending
    // failed-path retries — otherwise the entry would stay in the
    // queue until the element resumes or times out.
    orders.failed_path_requests.retain(|r| r.owner != owner);
}

impl EngineInner {
    fn stop_actor_orders_phase(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: crate::sequence::SequencePriority,
        include_pending: bool,
    ) {
        tracing::trace!(
            target: "parity_stop",
            ?owner,
            ?stop_priority,
            "engine stop_owner enter"
        );
        let owner_pos = self
            .world
            .entities
            .get(owner)
            .map(|e| e.element_data().position_map())
            .unwrap_or_default();
        if tracing::enabled!(target: "parity_owner_handoff", tracing::Level::TRACE) {
            let selected = self.world.entities.current_element_for_actor(owner);
            let selected_state = selected.and_then(|(seq_id, elem_idx)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| {
                        (
                            element.command,
                            element.state,
                            element.priority,
                            element
                                .orders
                                .front()
                                .map(|order| (order.order_type, order.order_id)),
                        )
                    })
            });
            let goal = self
                .world
                .entities
                .get(owner)
                .map(|entity| entity.position_iface().map_goal());
            tracing::trace!(
                target: "parity_owner_handoff",
                frame = self.control.frame_counter,
                ?owner,
                ?stop_priority,
                ?selected,
                ?selected_state,
                ?goal,
                "stop_owner before movement and sequence stop"
            );
        }
        let resolver = |engine: &EngineInner, element: &crate::sequence::SequenceElement| {
            Self::priority_resolver(&engine.world.entities)(element)
        };
        let selected_movement_before_stop = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
            .map(|(seq_id, elem_idx, order)| (seq_id, elem_idx, order.order_id));
        tracing::trace!(target: "parity_stop", ?owner, "before stop_movement_for_owner");
        self.stop_movement_for_owner(
            sim,
            assets,
            active_scripts,
            owner,
            owner_pos,
            stop_priority,
            &resolver,
        );
        let rewritten_selected_order = if let Some((before_seq, before_idx, before_id)) =
            selected_movement_before_stop
            && let Some((after_seq, after_idx, after_order)) = self
                .orders
                .sequence_manager
                .current_order_for_actor(&self.world.entities, owner)
            && after_seq == before_seq
            && after_idx == before_idx
            && after_order.order_id != before_id
        {
            // Stopping movement mutates the first
            // the order's action and assigns a new ID in place. The actor order still points
            // at that same object, so update Rust's explicit pointer mirror
            // only when the selected element survived with a rewritten ID.
            Some(crate::element::InstalledActorOrder {
                order_id: after_order.order_id,
                order_type: after_order.order_type,
            })
        } else {
            None
        };
        tracing::trace!(target: "parity_stop", ?owner, "after stop_movement_for_owner");
        // Path-request cleanup pairs cancellation with
        // failed-path-retry removal whenever a movement element
        // transitions out of MOVE_WAITING.  Mirror that here so a
        // `stop_owner` tear-down also evicts any stale retry entries
        // for this actor — otherwise the 100-frame timeout would fire
        // `element_impossible` / hero-speech on a sequence that no
        // longer cares.
        self.orders
            .failed_path_requests
            .retain(|r| r.owner != owner);
        self.orders.pending_path_requests.cancel_for_owner(owner);
        if let Some(installed_order) = rewritten_selected_order {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("rewritten movement-stop owner lost actor data")
                .installed_order = Some(installed_order);
        }
        tracing::trace!(target: "parity_stop", ?owner, "before sequence stop_owner");
        if include_pending {
            self.stop_owner(sim, assets, active_scripts, owner, stop_priority, &resolver);
        } else {
            let root = self.world.entities.current_element_for_actor(owner);
            self.stop_owner_current_from_root(
                sim,
                assets,
                active_scripts,
                root,
                stop_priority,
                &resolver,
            );
        }

        tracing::trace!(target: "parity_stop", ?owner, "after sequence stop_owner");
        tracing::trace!(
            target: "parity_stop",
            ?owner,
            ?stop_priority,
            "engine stop_owner exit"
        );
    }
}

#[cfg(test)]
mod tests;
