//! Sequence manager callbacks responsibilities.
use super::*;
use crate::engine::TickCtx;

impl SequenceManager {
    // ─── State change callbacks ─────────────────────────────────

    /// Resolve a retained element's priority for Stop without bypassing the
    /// manager's priority-dependent caches. Stop promotes a resolver's `None`
    /// to `Normal`; ordinary instruction-time resolution deliberately does not.
    pub(crate) fn resolve_element_stop_priority(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) -> SequencePriority {
        let element = self.get_element(seq_id, elem_idx).unwrap_or_else(|| {
            panic!("cannot resolve Stop priority for missing element {seq_id:?}/{elem_idx}")
        });
        if element.priority != SequencePriority::NotYetSet {
            return element.priority;
        }
        let resolved = match resolver(element) {
            SequencePriority::None => SequencePriority::Normal,
            priority => priority,
        };
        self.set_element_priority(seq_id, elem_idx, resolved);
        resolved
    }

    /// Set the priority of a specific sequence element.
    ///
    /// Used by the falling-pushed / rolling / ladder-wall / landing
    /// dispatch paths to mark the active element `NonInterruptable`
    /// so the termination guard in `element_impossible` refuses to
    /// cut it short.
    pub fn set_element_priority(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        priority: SequencePriority,
    ) {
        if let Some(seq) = self.sequences.get_mut(&seq_id)
            && let Some(elem) = seq.elements.get_mut(elem_idx)
        {
            let old_priority = elem.priority;
            let owner = elem.owner;
            let is_live = Self::is_actor_live_state(elem.state);
            elem.priority = priority;
            if is_live
                && old_priority != priority
                && let Some(owner) = owner
            {
                self.invalidate_postpone_tail_cache_for(owner);
            }
        }
    }

    /// Re-register shoulder-climb interactions that were held while their
    /// antagonist was still entering the helping-climb posture.
    ///
    /// Unlike ordinary actor-priority postponement this dependency crosses
    /// actors, so it cannot use the owner's `cross_postponed` chain. The
    /// helper's entry-element termination is the release boundary; retaining
    /// the interaction itself as `Postponed` keeps the relationship fully in
    /// authoritative sequence state without adding a parallel pending queue.
    pub(crate) fn resume_postponed_climbs_for_helper(&mut self, helper: EntityId) {
        let pending: Vec<_> =
            self.sequences
                .iter()
                .flat_map(|(&sequence_id, sequence)| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            let targets_helper = matches!(
                                &element.data,
                                SequenceElementData::Interaction {
                                    antagonist: Some(antagonist),
                                } if *antagonist == helper
                            );
                            (element.state == SequenceState::Postponed
                                && element.command == Command::ClimbUpOnShoulders
                                && targets_helper)
                                .then_some((sequence_id, element_index))
                        },
                    )
                })
                .collect();

        for (sequence_id, element_index) in pending {
            let element = self
                .sequences
                .get_mut(&sequence_id)
                .and_then(|sequence| sequence.elements.get_mut(element_index))
                .unwrap_or_else(|| {
                    panic!("postponed shoulder climb {sequence_id:?}/{element_index} disappeared")
                });
            // Re-instruction is a fresh actor-instruction boundary. Match the
            // ordinary cross-sequence postponed release below: arbitration
            // accepts only Todo work, and transition generation must resample
            // the climber's live posture rather than reuse the stamp from the
            // original, premature instruction.
            element.state = SequenceState::Todo;
            element.posture_after_transition = crate::element::Posture::Undefined;
            self.elements_to_go.push_back((sequence_id, element_index));
        }
    }

    /// Whether the front order on the given element can be interrupted
    /// right now.
    ///
    /// A current order must exist and is always interruptible. The order's AI-lock flag
    /// is serialized but is never consulted by sequence arbitration. Keep the
    /// field for save compatibility without inventing gameplay semantics for
    /// it here.
    pub fn can_interrupt_now(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        let elem = self.get_element(seq_id, elem_idx).unwrap_or_else(|| {
            panic!("can_interrupt_now called for missing sequence element {seq_id:?}/{elem_idx}")
        });
        assert!(
            elem.orders.front().is_some(),
            "can_interrupt_now requires a current order on {seq_id:?}/{elem_idx}"
        );
        true
    }

    pub(super) fn invalidate_postpone_tail_cache_for(&mut self, owner: EntityId) {
        let append_entries = self
            .postpone_tail_cache
            .remove(&owner)
            .map_or(0, |entries| entries.len());
        tracing::trace!(
            target: "parity_stop_cache",
            ?owner,
            append_entries,
            reason = "topology_or_priority",
            "invalidate owner chain caches"
        );
    }

    /// Return the first blocker at which priority arbitration is not the pure
    /// `Postpone` tail-call arm. If every existing successor chooses
    /// `Postpone`, this is the chain tail and the result is cached.
    pub(crate) fn postpone_append_point(
        &mut self,
        root: (SequenceId, usize),
        waiter_priority: SequencePriority,
    ) -> ((SequenceId, usize), usize, bool) {
        let root_ref = SequenceElementRef::new(root.0, root.1);
        let owner = self
            .get_element(root.0, root.1)
            .and_then(|element| element.owner)
            .unwrap_or_else(|| {
                panic!(
                    "postpone chain root {:?}/{} is missing or ownerless",
                    root.0, root.1
                )
            });
        if let Some(&summary) = self
            .postpone_tail_cache
            .get(&owner)
            .and_then(|cache| cache.get(&(root_ref, waiter_priority)))
        {
            let tail = summary.tail;
            let tail_element = self
                .get_element(tail.sequence_id, tail.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "stale postpone-tail cache references missing {:?}/{}",
                        tail.sequence_id, tail.element_index
                    )
                });
            assert_eq!(
                tail_element.owner,
                Some(owner),
                "postpone-tail cache crosses owners"
            );
            assert!(
                tail_element.postponed.is_none(),
                "postpone-tail cache was not invalidated before {:?}/{} changed",
                tail.sequence_id,
                tail.element_index
            );
            return ((tail.sequence_id, tail.element_index), summary.hops, true);
        }

        let mut current = root;
        let mut hops = 0;
        let mut visited = HashSet::new();
        loop {
            assert!(
                visited.insert(current),
                "cross-postponed cycle while locating append point at {:?}/{}",
                current.0,
                current.1
            );
            let element = self.get_element(current.0, current.1).unwrap_or_else(|| {
                panic!(
                    "cross-postponed chain references missing {:?}/{}",
                    current.0, current.1
                )
            });
            assert_eq!(
                element.owner,
                Some(owner),
                "cross-postponed chain crosses owners at {:?}/{}",
                current.0,
                current.1
            );
            let Some(next) = element
                .postponed
                .map(|reference| (reference.sequence_id, reference.element_index))
            else {
                self.postpone_tail_cache.entry(owner).or_default().insert(
                    (root_ref, waiter_priority),
                    PostponeTailSummary {
                        tail: SequenceElementRef::new(current.0, current.1),
                        hops,
                    },
                );
                return (current, hops, true);
            };
            let existing_priority = self
                .get_element(next.0, next.1)
                .unwrap_or_else(|| {
                    panic!(
                        "cross-postponed chain references missing {:?}/{}",
                        next.0, next.1
                    )
                })
                .priority;
            if decide_priorities(existing_priority, waiter_priority) != PriorityDecision::Postpone {
                return (current, hops, false);
            }
            current = next;
            hops += 1;
        }
    }

    /// Install an append discovered by [`Self::postpone_append_point`] and
    /// advance that exact root/priority cache entry to the new tail.
    pub(crate) fn install_cached_postpone_append(
        &mut self,
        root: (SequenceId, usize),
        waiter_priority: SequencePriority,
        blocker: (SequenceId, usize),
        waiter: (SequenceId, usize),
        prior_hops: usize,
    ) {
        let owner = self
            .get_element(blocker.0, blocker.1)
            .and_then(|element| element.owner)
            .expect("postpone append blocker is missing or ownerless");
        let waiter_owner = self
            .get_element(waiter.0, waiter.1)
            .and_then(|element| element.owner)
            .expect("postpone append waiter is missing or ownerless");
        assert_eq!(owner, waiter_owner, "postpone append crosses owners");
        let prior_summary = *self
            .postpone_tail_cache
            .get(&owner)
            .and_then(|cache| {
                cache.get(&(SequenceElementRef::new(root.0, root.1), waiter_priority))
            })
            .expect("cacheable postpone append lost its root summary");
        assert_eq!(
            prior_summary.tail,
            SequenceElementRef::new(blocker.0, blocker.1),
            "cacheable postpone append blocker is not the cached tail"
        );
        assert_eq!(prior_summary.hops, prior_hops);
        let waiter_has_no_cross_successor = self
            .get_element(waiter.0, waiter.1)
            .is_some_and(|element| element.postponed.is_none());
        self.invalidate_postpone_tail_cache_for(owner);
        let blocker_element = self
            .get_element_mut(blocker.0, blocker.1)
            .expect("postpone append blocker disappeared");
        assert!(
            blocker_element.postponed.is_none(),
            "postpone append point already has a successor"
        );
        blocker_element.postponed = Some(SequenceElementRef::new(waiter.0, waiter.1));
        // A waiter may already own a postponed successor chain (for example
        // after adopting postponed work). In that case it is not the new tail, so
        // caching it as one would leave a stale summary immediately after
        // this append. Let the next lookup walk the complete chain instead.
        if waiter_has_no_cross_successor
            && decide_priorities(waiter_priority, waiter_priority) == PriorityDecision::Postpone
        {
            self.postpone_tail_cache.entry(owner).or_default().insert(
                (SequenceElementRef::new(root.0, root.1), waiter_priority),
                PostponeTailSummary {
                    tail: SequenceElementRef::new(waiter.0, waiter.1),
                    hops: prior_hops + 1,
                },
            );
        }
    }

    pub(crate) fn set_cross_postponed_link(
        &mut self,
        blocker: (SequenceId, usize),
        successor: Option<(SequenceId, usize)>,
    ) {
        let owner = self
            .get_element(blocker.0, blocker.1)
            .and_then(|element| element.owner)
            .expect("cross-postponed blocker is missing or ownerless");
        self.invalidate_postpone_tail_cache_for(owner);
        self.get_element_mut(blocker.0, blocker.1)
            .expect("cross-postponed blocker disappeared")
            .postponed =
            successor.map(|(sequence, index)| SequenceElementRef::new(sequence, index));
    }

    /// Transfer a cross-sequence postponed successor from `src` onto
    /// `dst`, walking `dst`'s existing postponed chain to the tail if
    /// it already has one.
    pub fn take_over_postponed(
        &mut self,
        dst_seq: SequenceId,
        dst_idx: usize,
        src_seq: SequenceId,
        src_idx: usize,
    ) {
        let Some(src_next) = self.get_element(src_seq, src_idx).and_then(|e| {
            e.postponed
                .map(|reference| (reference.sequence_id, reference.element_index))
        }) else {
            return;
        };
        // Walk dst's chain to the tail (first element with no
        // cross_postponed).  At most `sequences.len()` hops — the chain
        // is acyclic by construction.
        let mut cur = (dst_seq, dst_idx);
        loop {
            let Some(e) = self.get_element(cur.0, cur.1) else {
                return;
            };
            match e.postponed {
                None => break,
                Some(next) => cur = (next.sequence_id, next.element_index),
            }
        }
        // Install src's successor at the tail.
        self.set_cross_postponed_link(cur, Some(src_next));
        self.set_cross_postponed_link((src_seq, src_idx), None);
    }
}

impl crate::engine::EngineInner {
    pub(crate) fn set_sequence_element_state(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
        state: SequenceState,
        flags: CascadeFlags,
        terminal_site: &'static str,
    ) {
        let element = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("sequence transition element missing");
        if element.state == state {
            return;
        }
        let movement_override =
            matches!(state, SequenceState::Interrupted | SequenceState::Postponed)
                && element.data.is_movement();
        if movement_override {
            if element.command == Command::MoveWaiting {
                let owner = element.owner.expect("waiting movement has no owner");
                self.orders.pending_path_requests.cancel_for_owner(owner);
                self.orders
                    .failed_path_requests
                    .retain(|request| request.owner != owner);
                self.orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .expect("waiting movement disappeared")
                    .command = Command::Move;
            }
            if state == SequenceState::Postponed {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .expect("postponed movement disappeared");
                if element.command == Command::MoveOk {
                    element.command = Command::Move;
                }
            }
        }
        if movement_override && state == SequenceState::Interrupted {
            let movement = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .expect("interrupted movement disappeared");
            let SequenceElementData::Movement {
                linked_seek: linked,
                ..
            } = movement.data
            else {
                unreachable!("interrupted movement changed subtype");
            };
            if let Some(linked) = linked {
                assert!(
                    self.orders
                        .sequence_manager
                        .get_element(linked.sequence_id, linked.element_index)
                        .expect("linked Seek missing")
                        .data
                        .is_movement(),
                    "linked Seek is not movement"
                );
                self.element_interrupted(
                    tcx,
                    active_scripts,
                    linked.sequence_id,
                    linked.element_index,
                    CascadeFlags::FOLLOWING,
                );
            }
        }
        // Linked movement callbacks finish before the base transition reads state.
        let (old_state, owner) = {
            let sequence = self
                .orders
                .sequence_manager
                .sequences
                .get_mut(&seq_id)
                .expect("sequence disappeared during state transition");
            let element = &mut sequence.elements[elem_idx];
            let old_state = element.state;
            if old_state == state {
                return;
            }
            element.state = state;
            let owner = element.owner;
            if state == SequenceState::InProgress {
                sequence.increase_elements_in_progress();
            } else if old_state == SequenceState::InProgress {
                sequence.decrease_elements_in_progress();
            }
            (old_state, owner)
        };
        if let Some(owner) = owner {
            let element_ref = SequenceElementRef::new(seq_id, elem_idx);
            match (
                SequenceManager::is_actor_live_state(old_state),
                SequenceManager::is_actor_live_state(state),
            ) {
                (false, true) => self
                    .orders
                    .sequence_manager
                    .insert_actor_live_ref(owner, element_ref),
                (true, false) => self
                    .orders
                    .sequence_manager
                    .remove_actor_live_ref(owner, element_ref),
                _ => {}
            }
        }
        match state {
            SequenceState::InProgress => {
                debug_assert!(
                    matches!(old_state, SequenceState::Todo | SequenceState::Postponed),
                    "InProgress from {:?}",
                    old_state
                );
            }
            SequenceState::Impossible | SequenceState::Interrupted => {
                if state == SequenceState::Impossible {
                    self.start_postponed_sequence_element(tcx, active_scripts, seq_id, elem_idx);
                }
                self.orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .expect("terminal element disappeared")
                    .orders
                    .clear();
                self.notify_sequence_element_owner(tcx, seq_id, elem_idx, state, terminal_site);
                // Owner callbacks can replace the following edge and its command level.
                if let Some(target) = self
                    .orders
                    .sequence_manager
                    .live_cascade_target(seq_id, elem_idx, flags)
                {
                    self.set_sequence_element_state(
                        tcx,
                        active_scripts,
                        target.sequence_id,
                        target.element_index,
                        state,
                        CascadeFlags::FOLLOWING,
                        "terminal_state_cascade",
                    );
                }
            }
            SequenceState::Terminated => {
                if matches!(
                    old_state,
                    SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed
                ) {
                    self.notify_sequence_element_owner(tcx, seq_id, elem_idx, state, terminal_site);
                    self.sequence_element_ready(tcx, active_scripts, seq_id);
                    self.start_postponed_sequence_element(tcx, active_scripts, seq_id, elem_idx);
                } else {
                    tracing::warn!(
                        sequence_id = seq_id.0,
                        element_index = elem_idx,
                        ?old_state,
                        "sequence element terminated from a shipping-only state"
                    );
                }
            }
            SequenceState::Postponed | SequenceState::Done | SequenceState::Todo => {}
        }
    }

    /// Called by the engine when an element has finished (terminated).
    /// Advances the sequence to the next command level if all elements at
    /// the current level are done.
    #[track_caller]
    pub(crate) fn element_terminated(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        tracing::trace!(
            target: "parity_terminate_caller",
            ?seq_id,
            elem_idx,
            caller = %std::panic::Location::caller(),
            "element_terminated"
        );
        if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
            return;
        }

        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::Terminated,
            CascadeFlags::NEXT_LEVEL,
            "element_terminated",
        );
    }

    /// Called when an element becomes impossible.
    ///
    /// Sequence elements marked `SequencePriority::NonInterruptable`
    /// must run to completion and can't be downgraded to `Impossible`
    /// by external events. When something tries, the call is logged
    /// and treated as a no-op so the element stays `InProgress` and
    /// finishes normally.
    pub(crate) fn element_impossible(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        let Some(seq) = self.orders.sequence_manager.sequences.get(&seq_id) else {
            return;
        };

        // Priority guard: non-interruptable elements ignore "impossible"
        // downgrades from outside their natural completion path.
        let elem = seq
            .elements
            .get(elem_idx)
            .unwrap_or_else(|| panic!("missing impossible element {seq_id:?}/{elem_idx}"));
        let blocked =
            elem.state == SequenceState::InProgress && elem.priority.is_non_interruptable();
        if blocked {
            tracing::debug!(
                ?seq_id,
                elem_idx,
                "element_impossible: blocked by NonInterruptable priority — keeping element in progress"
            );
            return;
        }

        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
            "element_impossible",
        );
    }

    /// Apply an aborted-motion result returned by an actor's own
    /// `Execute` call.
    ///
    /// This is distinct from an external attempt to invalidate an active
    /// element. The actor update asserts in debug builds that its
    /// retained element is not non-interruptable, but release builds still
    /// mark the sequence impossible after an intrinsic execution abort.
    /// Preserve that release behavior for malformed/sentinel orders authored
    /// by Original itself.
    pub(crate) fn element_impossible_from_execute(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
            return;
        }

        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
            "element_impossible_from_execute",
        );
    }

    /// Called when an element starts executing (enters InProgress).
    pub(crate) fn element_in_progress(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
            return;
        }

        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::InProgress,
            CascadeFlags::NEXT_LEVEL,
            "element_in_progress",
        );
    }

    /// Called when an element is interrupted.
    pub(crate) fn element_interrupted(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
            return;
        }

        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::Interrupted,
            flags,
            "element_interrupted",
        );
    }

    /// Hard-interrupt every live sequence element owned by `actor`, except
    /// those in `exempt_seq` and dead-admissible cards already waiting in the
    /// FIFO.
    ///
    /// Used on death: the graceful `stop_owner` path rewrites an
    /// in-progress movement order to a `TransitionWalking*Waiting*` stop
    /// animation and lets the element keep playing — which is correct
    /// for a live halt but produces a "corpse walks a few more frames"
    /// visual for a dead actor.  Death needs to throw every surviving
    /// sequence away cleanly. Original-game human death does not purge its
    /// sequence queue, and dead-human instruction handling still admits the five
    /// ordinary damage-reception commands, waiting, and death at the bottom. Preserve
    /// those `Todo` cards so simultaneous hits execute in FIFO order after the
    /// lethal hit; the active damage sequence survives via `exempt_seq` so its
    /// dying order becomes the actor's current order.
    ///
    /// Our arbitration doesn't run on state changes, so we do the
    /// cleanup explicitly here.
    pub(crate) fn kill_owner_sequences(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        actor: EntityId,
        exempt_seq: Option<SequenceId>,
    ) {
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for (seq_id, seq) in &self.orders.sequence_manager.sequences {
            if Some(*seq_id) == exempt_seq {
                continue;
            }
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                if elem.owner != Some(actor) {
                    continue;
                }
                let pending_command_admitted_while_dead =
                    matches!(elem.state, SequenceState::Todo | SequenceState::Postponed)
                        && matches!(
                            elem.command,
                            Command::ReceiveHitDamage
                                | Command::ReceiveSwordDamage
                                | Command::ReceiveArrowDamage
                                | Command::ReceiveDamage
                                | Command::ReceiveMobileDamage
                                | Command::Wait
                                | Command::GetKilledAtBottom
                        );
                if pending_command_admitted_while_dead {
                    continue;
                }
                if matches!(
                    elem.state,
                    SequenceState::InProgress | SequenceState::Postponed | SequenceState::Todo
                ) {
                    targets.push((*seq_id, elem_idx));
                }
            }
        }
        for (seq_id, elem_idx) in targets {
            if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
                continue;
            }
            self.set_sequence_element_state(
                tcx,
                active_scripts,
                seq_id,
                elem_idx,
                SequenceState::Interrupted,
                CascadeFlags::NEXT_LEVEL,
                "kill_owner_sequences",
            );
        }
    }

    /// Postpone after arbitration installs the graph links. Movement cancellation
    /// and progress accounting happen synchronously without notifying the owner.
    pub(crate) fn postpone_element(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        if !self.orders.sequence_manager.sequences.contains_key(&seq_id) {
            return;
        }
        self.set_sequence_element_state(
            tcx,
            active_scripts,
            seq_id,
            elem_idx,
            SequenceState::Postponed,
            CascadeFlags::empty(),
            "postpone_element",
        );

        // The original game postpones from inside the element's instruction path
        // boundary, after the sequence-manager tick has already removed
        // that element from its launch FIFO. Rust also arbitrates owned
        // launches synchronously, while their initial manager registration
        // is still queued. Consume that registration here: otherwise the
        // manager instructs the same postponed element again next frame and
        // can attach it behind itself, creating a recursive self-cycle.
        let target = (seq_id, elem_idx);
        self.orders
            .sequence_manager
            .elements_to_go
            .retain(|entry| *entry != target);
    }

    fn notify_sequence_element_owner(
        &mut self,
        tcx: TickCtx<'_>,
        seq_id: SequenceId,
        elem_idx: usize,
        terminal_state: SequenceState,
        terminal_site: &'static str,
    ) {
        let element = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("owner notification element missing");
        let Some(owner) = element.owner else {
            return;
        };
        let card = CondolationCard {
            owner,
            command: element.command,
            terminal_state,
            seq_id,
            elem_idx: elem_idx as u16,
            from_halt: self.orders.sequence_manager.halt_pending,
        };
        if goal_owner_debug_matches(owner) {
            let provenance = GoalOwnerTerminalProvenance {
                site: terminal_site,
                selected: self.world.entities.current_element_for_actor(owner),
            };
            GOAL_OWNER_TERMINAL_PROVENANCE.with(|records| {
                records
                    .borrow_mut()
                    .insert((seq_id, card.elem_idx), provenance);
            });
        }
        tracing::trace!(
            target: "parity_owner_handoff", ?owner, ?seq_id, elem_idx,
            command = ?card.command, ?terminal_state,
            selected = ?self.world.entities.current_element_for_actor(owner),
            "removal notification capturing selection at state change"
        );
        self.send_condolation_card(tcx, card);
    }

    fn sequence_element_ready(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
    ) {
        let to_go = {
            let Some(seq) = self.orders.sequence_manager.sequences.get_mut(&seq_id) else {
                return;
            };
            if seq.running_elements == 0 {
                let elements: Vec<_> = seq
                    .elements
                    .iter()
                    .enumerate()
                    .map(|(idx, elem)| {
                        (
                            idx,
                            elem.command,
                            elem.command_level,
                            elem.owner,
                            elem.state,
                            elem.priority,
                            elem.orders.len(),
                        )
                    })
                    .collect();
                panic!(
                    "Ready called with no running elements: seq_id={seq_id:?} cursor={} current_level={} elements_in_progress={} elements={elements:?}",
                    seq.cursor, seq.current_command_level, seq.elements_in_progress
                );
            }
            if seq.element_ready() {
                seq.next_elements_go()
            } else {
                Vec::new()
            }
        };
        self.register_sequence_level(tcx, active_scripts, seq_id, to_go)
            .unwrap_or_else(|error| panic!("sequence Ready failed: {error:?}"));
    }

    fn start_postponed_sequence_element(
        &mut self,
        tcx: TickCtx<'_>,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        seq_id: SequenceId,
        anchor: usize,
    ) {
        let target = self
            .orders
            .sequence_manager
            .sequences
            .get(&seq_id)
            .and_then(|sequence| sequence.live_postponed_ref(anchor));
        let Some(target) = target else {
            return;
        };
        let source_owner = self
            .orders
            .sequence_manager
            .get_element(seq_id, anchor)
            .expect("postponed source missing")
            .owner;
        let successor = self
            .orders
            .sequence_manager
            .get_element(target.sequence_id, target.element_index)
            .expect("postponed successor missing");
        if successor.command == Command::MoveOk && successor.owner.is_some() {
            let owner = source_owner.expect("movement blocker has no owner");
            let target_owner = successor.owner.expect("postponed movement owner missing");
            if self
                .world
                .entities
                .get(owner)
                .expect("movement blocker owner missing")
                .posture()
                != self
                    .world
                    .entities
                    .get(target_owner)
                    .expect("postponed movement owner missing")
                    .posture()
            {
                let successor = self
                    .orders
                    .sequence_manager
                    .get_element_mut(target.sequence_id, target.element_index)
                    .expect("postponed successor missing");
                successor.orders.clear();
                successor.command = Command::Move;
            }
        }
        self.register_sequence_element(
            tcx,
            active_scripts,
            target.sequence_id,
            target.element_index,
            false,
        )
        .unwrap_or_else(|error| panic!("postponed registration failed: {error:?}"));
        self.orders
            .sequence_manager
            .sequences
            .get_mut(&seq_id)
            .expect("postponed source disappeared")
            .sever_postponed_link(anchor);
    }
}
