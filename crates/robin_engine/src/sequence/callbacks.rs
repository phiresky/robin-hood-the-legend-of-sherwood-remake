//! Sequence manager callbacks responsibilities.
use super::*;

impl SequenceManager {
    // ─── State change callbacks ─────────────────────────────────

    /// Called by the engine when an element has finished (terminated).
    /// Advances the sequence to the next command level if all elements at
    /// the current level are done.
    #[track_caller]
    pub fn element_terminated(&mut self, seq_id: SequenceId, elem_idx: usize) {
        tracing::trace!(
            target: "parity_terminate_caller",
            ?seq_id,
            elem_idx,
            caller = %std::panic::Location::caller(),
            "element_terminated"
        );
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Terminated,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_terminated");
    }

    /// Called when an element becomes impossible.
    ///
    /// Sequence elements marked `SequencePriority::NonInterruptable`
    /// must run to completion and can't be downgraded to `Impossible`
    /// by external events. When something tries, the call is logged
    /// and treated as a no-op so the element stays `InProgress` and
    /// finishes normally.
    pub fn element_impossible(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
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

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_impossible");
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
    pub fn element_impossible_from_execute(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_impossible_from_execute");
    }

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
                if priority > old_priority {
                    if let Some(summary) = self.actor_stop_summaries.get_mut(&owner) {
                        summary.weakest_priority = summary.weakest_priority.max(priority);
                    }
                } else if self
                    .actor_stop_summaries
                    .get(&owner)
                    .is_some_and(|summary| summary.weakest_priority == old_priority)
                {
                    self.actor_stop_summaries.remove(&owner);
                }
            }
        }
    }

    /// Called when an element starts executing (enters InProgress).
    pub fn element_in_progress(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::InProgress,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_in_progress");
    }

    /// Called when an element is interrupted.
    pub fn element_interrupted(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(elem_idx, SequenceState::Interrupted, flags);

        self.process_effects(seq_id, effects, "element_interrupted");
    }

    /// Interrupt an element after its actor has already selected an incoming
    /// replacement.
    ///
    /// The original game selects the new sequence element before it interrupts the old
    /// element.  Consequently the old element's synchronous
    /// removal notification observes that it is no longer selected. Rust's
    /// incoming element is still `Todo` at this borrow-safe boundary, so the
    /// actor-in-progress index alone would incorrectly mark the old card as
    /// selected.
    pub fn element_interrupted_after_replacement_selected(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        let pending_before = self.pending_condolations.len();
        self.element_interrupted(seq_id, elem_idx, flags);
        let dispatch = self
            .pending_condolations
            .get_mut(pending_before)
            .expect("replacement interruption must queue its condolence card");
        assert_eq!(dispatch.card.seq_id, seq_id);
        assert_eq!(usize::from(dispatch.card.elem_idx), elem_idx);
        dispatch.card.was_selected = false;
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
    pub fn kill_owner_sequences(&mut self, actor: EntityId, exempt_seq: SequenceId) {
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for (seq_id, seq) in &self.sequences {
            if *seq_id == exempt_seq {
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
            let Some(seq) = self.sequences.get_mut(&seq_id) else {
                continue;
            };
            let effects = seq.set_element_state(
                elem_idx,
                SequenceState::Interrupted,
                CascadeFlags::NEXT_LEVEL,
            );
            self.process_effects(seq_id, effects, "kill_owner_sequences");
        }
    }

    /// Flip an element to `Postponed` via the normal state-change
    /// pipeline. Used by the instruction arbitration path. The common
    /// `set_element_state` prologue still runs (so the in-progress
    /// counter decrements when the waiter was InProgress), while the
    /// `Postponed` case body itself does nothing extra — no cascade,
    /// no signal_ready, no condolation.  `CascadeFlags::empty()`
    /// reflects that, and `process_effects` keeps `actor_in_progress`
    /// / `elements_in_progress` consistent on the InProgress→Postponed
    /// transition.  The element's `cross_postponed` / `postponed_by`
    /// links are set separately by the caller before this call.
    pub fn postpone_element(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };
        let effects =
            seq.set_element_state(elem_idx, SequenceState::Postponed, CascadeFlags::empty());
        self.process_effects(seq_id, effects, "postpone_element");

        // The original game postpones from inside the element's instruction path
        // boundary, after the sequence-manager tick has already removed
        // that element from its launch FIFO. Rust also arbitrates owned
        // launches synchronously, while their initial manager registration
        // is still queued. Consume that registration here: otherwise the
        // manager instructs the same postponed element again next frame and
        // can attach it behind itself, creating a recursive self-cycle.
        let target = (seq_id, elem_idx);
        self.elements_to_go.retain(|entry| *entry != target);
        self.pending_synchronous_actions.retain(|entry| {
            !matches!(
                entry.as_action(),
                Some(SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }) if (*sequence_id, *element_index) == target
            )
        });
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
            self.register_element_to_go(sequence_id, element_index);
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
        let stop_entries = self
            .stop_noop_cache
            .remove(&owner)
            .map_or(0, |entries| entries.len());
        tracing::trace!(
            target: "parity_stop_cache",
            ?owner,
            append_entries,
            stop_entries,
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
                tail_element.cross_postponed.is_none(),
                "postpone-tail cache was not invalidated before {:?}/{} changed",
                tail.sequence_id,
                tail.element_index
            );
            return ((tail.sequence_id, tail.element_index), summary.hops, true);
        }

        let mut current = root;
        let mut hops = 0;
        let mut weakest_priority = SequencePriority::NonInterruptable;
        let mut cross_only = true;
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
            weakest_priority = weakest_priority.max(element.priority);
            cross_only &= self
                .get_sequence(current.0)
                .and_then(|sequence| sequence.following_element_index(current.1))
                .is_none()
                && element.postponed_element_index.is_none();
            let Some(next) = element.cross_postponed else {
                self.postpone_tail_cache.entry(owner).or_default().insert(
                    (root_ref, waiter_priority),
                    PostponeTailSummary {
                        tail: SequenceElementRef::new(current.0, current.1),
                        hops,
                        weakest_priority,
                        cross_only,
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
        let waiter_cross_only = self
            .get_sequence(waiter.0)
            .and_then(|sequence| sequence.following_element_index(waiter.1))
            .is_none()
            && self
                .get_element(waiter.0, waiter.1)
                .is_some_and(|element| element.postponed_element_index.is_none());
        let waiter_has_no_cross_successor = self
            .get_element(waiter.0, waiter.1)
            .is_some_and(|element| element.cross_postponed.is_none());
        let root_ref = SequenceElementRef::new(root.0, root.1);
        let preserved_stop_priorities = self
            .stop_noop_cache
            .get(&owner)
            .into_iter()
            .flat_map(|entries| entries.iter())
            .filter_map(|((cached_root, stop_priority), summary)| {
                if *cached_root != root_ref {
                    return None;
                }
                assert_eq!(
                    summary.tail,
                    SequenceElementRef::new(blocker.0, blocker.1),
                    "cached no-op Stop tail disagrees with append tail"
                );
                (waiter_cross_only
                    && waiter_has_no_cross_successor
                    && waiter_priority < *stop_priority)
                    .then_some(*stop_priority)
            })
            .collect::<Vec<_>>();
        self.invalidate_postpone_tail_cache_for(owner);
        let blocker_element = self
            .get_element_mut(blocker.0, blocker.1)
            .expect("postpone append blocker disappeared");
        assert!(
            blocker_element.cross_postponed.is_none(),
            "postpone append point already has a successor"
        );
        blocker_element.cross_postponed = Some(waiter);
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
                    weakest_priority: prior_summary.weakest_priority.max(waiter_priority),
                    cross_only: prior_summary.cross_only && waiter_cross_only,
                },
            );
        }
        for stop_priority in preserved_stop_priorities {
            self.stop_noop_cache.entry(owner).or_default().insert(
                (root_ref, stop_priority),
                StopNoopSummary {
                    tail: SequenceElementRef::new(waiter.0, waiter.1),
                },
            );
        }
    }

    pub(super) fn selected_cross_chain_all_stronger(
        &self,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
    ) -> bool {
        let Some(owner) = self
            .get_element(root.0, root.1)
            .and_then(|element| element.owner)
        else {
            return false;
        };
        let root_ref = SequenceElementRef::new(root.0, root.1);
        let Some(summary) = self.postpone_tail_cache.get(&owner).and_then(|cache| {
            cache.iter().find_map(|((cached_root, _), summary)| {
                (*cached_root == root_ref).then_some(*summary)
            })
        }) else {
            return false;
        };
        let tail = self
            .get_element(summary.tail.sequence_id, summary.tail.element_index)
            .unwrap_or_else(|| {
                panic!(
                    "selected Stop cache references missing tail {:?}/{}",
                    summary.tail.sequence_id, summary.tail.element_index
                )
            });
        assert!(
            tail.cross_postponed.is_none(),
            "selected Stop cache was not invalidated before its tail changed"
        );
        summary.cross_only && summary.weakest_priority < stop_priority
    }

    pub(super) fn selected_stop_is_cached_noop(
        &self,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
    ) -> bool {
        let Some(owner) = self
            .get_element(root.0, root.1)
            .and_then(|element| element.owner)
        else {
            return false;
        };
        let Some(summary) = self.stop_noop_cache.get(&owner).and_then(|entries| {
            entries.get(&(SequenceElementRef::new(root.0, root.1), stop_priority))
        }) else {
            return false;
        };
        let tail = self
            .get_element(summary.tail.sequence_id, summary.tail.element_index)
            .unwrap_or_else(|| {
                panic!(
                    "cached no-op Stop references missing tail {:?}/{}",
                    summary.tail.sequence_id, summary.tail.element_index
                )
            });
        assert!(
            tail.cross_postponed.is_none(),
            "cached no-op Stop was not invalidated before its tail changed"
        );
        true
    }

    pub(super) fn repair_selected_stop_noop(
        &mut self,
        owner: EntityId,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
    ) {
        let mut tail = root;
        let mut visited = HashSet::new();
        loop {
            assert!(
                visited.insert(tail),
                "cross-postponed cycle while caching no-op Stop at {:?}/{}",
                tail.0,
                tail.1
            );
            let element = self.get_element(tail.0, tail.1).unwrap_or_else(|| {
                panic!(
                    "no-op Stop graph references missing {:?}/{}",
                    tail.0, tail.1
                )
            });
            assert_eq!(
                element.owner,
                Some(owner),
                "no-op Stop graph crosses owners"
            );
            let Some(next) = element.cross_postponed else {
                break;
            };
            tail = next;
        }
        self.stop_noop_cache.entry(owner).or_default().insert(
            (SequenceElementRef::new(root.0, root.1), stop_priority),
            StopNoopSummary {
                tail: SequenceElementRef::new(tail.0, tail.1),
            },
        );
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
            .cross_postponed = successor;
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
        let Some(src_next) = self
            .get_element(src_seq, src_idx)
            .and_then(|e| e.cross_postponed)
        else {
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
            match e.cross_postponed {
                None => break,
                Some(next) => cur = next,
            }
        }
        // Install src's successor at the tail.
        self.set_cross_postponed_link(cur, Some(src_next));
        self.set_cross_postponed_link((src_seq, src_idx), None);
    }

    /// Process effects from a state change.
    pub(super) fn process_effects(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        terminal_site: &'static str,
    ) {
        self.process_effects_with_cross_cleanup(seq_id, effects, terminal_site, true);
    }

    /// Process a state transition while leaving inbound cross-postponed links
    /// for a caller-owned batch cleanup. Stopping a sequence element can walk a
    /// chain containing thousands of separately allocated sequences; scanning
    /// the complete manager after every node makes that linear graph
    /// quadratic in the number of retained sequences.
    pub(super) fn process_effects_deferring_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        terminal_site: &'static str,
    ) {
        self.process_effects_with_cross_cleanup(seq_id, effects, terminal_site, false);
    }

    pub(super) fn process_effects_with_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        mut effects: StateChangeEffects,
        terminal_site: &'static str,
        clear_cross_links: bool,
    ) {
        if effects.resume_cross_postponed.is_some()
            && let Some((_, owner, _, _)) = effects.actor_live_transition
        {
            // `Sequence::set_element_state` already took this source's
            // cross-postponed pointer before returning its effects.
            self.invalidate_postpone_tail_cache_for(owner);
        }
        // The movement-sequence override interrupts its exact linked
        // Seek before delegating to the base element's Interrupted handling.
        // Process the cross-sequence target before bookkeeping or queuing the
        // movement element's own condolence card to preserve callback order.
        if let Some(linked) = effects.interrupt_linked_seek.take() {
            let cancel_before_linked_callback = effects
                .condolation
                .as_mut()
                .and_then(|card| card.cancel_path_request_owner.take());
            let linked_element = self
                .get_element(linked.sequence_id, linked.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "loaded movement linked Seek references missing element {:?}/{}",
                        linked.sequence_id, linked.element_index
                    )
                });
            assert!(
                linked_element.data.is_movement(),
                "loaded movement linked Seek target {:?}/{} is not a movement element",
                linked.sequence_id,
                linked.element_index
            );
            let mut linked_effects = self
                .sequences
                .get_mut(&linked.sequence_id)
                .expect("linked Seek sequence disappeared")
                .set_element_state(
                    linked.element_index,
                    SequenceState::Interrupted,
                    CascadeFlags::FOLLOWING,
                );
            if let Some(cancel_owner) = cancel_before_linked_callback {
                if let Some(linked_card) = linked_effects.condolation.as_mut() {
                    assert!(
                        linked_card.cancel_path_request_owner.is_none()
                            || linked_card.cancel_path_request_owner == Some(cancel_owner),
                        "linked movement cancellation crosses actors"
                    );
                    linked_card.cancel_path_request_owner = Some(cancel_owner);
                } else if let Some(card) = effects.condolation.as_mut() {
                    // An already-terminal linked target has no callback. Keep
                    // cancellation on the source movement's own card.
                    card.cancel_path_request_owner = Some(cancel_owner);
                }
            }
            self.process_effects_with_cross_cleanup(
                linked.sequence_id,
                linked_effects,
                "linked_movement_seek_interrupt",
                clear_cross_links,
            );
        }

        if let Some(card) = effects.condolation.as_mut() {
            card.was_selected = self.current_element_for_actor(card.owner)
                == Some((card.seq_id, usize::from(card.elem_idx)));
            if goal_owner_debug_matches(card.owner) {
                let provenance = GoalOwnerTerminalProvenance {
                    site: terminal_site,
                    selected: self.current_element_for_actor(card.owner),
                    translating: self.actor_translating,
                };
                GOAL_OWNER_TERMINAL_PROVENANCE.with(|records| {
                    records
                        .borrow_mut()
                        .insert((card.seq_id, card.elem_idx), provenance);
                });
            }
            tracing::trace!(
                target: "parity_owner_handoff",
                owner = ?card.owner,
                seq_id = ?card.seq_id,
                elem_idx = card.elem_idx,
                command = ?card.command,
                terminal_state = ?card.terminal_state,
                was_selected = card.was_selected,
                instructing = ?self.actor_instructing.get(&card.owner),
                in_progress = ?self.actor_in_progress.get(&card.owner),
                "removal notification capturing selection at state change"
            );
        }

        if let Some(seq) = self.sequences.get_mut(&seq_id) {
            if effects.increment_in_progress {
                seq.increase_elements_in_progress();
            }
            if effects.decrement_in_progress {
                seq.decrease_elements_in_progress();
            }
        }

        if let Some((elem_idx, owner, old_state, new_state)) = effects.actor_live_transition {
            let elem_ref = SequenceElementRef::new(seq_id, elem_idx);
            match (
                Self::is_actor_live_state(old_state),
                Self::is_actor_live_state(new_state),
            ) {
                (false, true) => self.insert_actor_live_ref(owner, elem_ref),
                (true, false) => self.remove_actor_live_ref(owner, elem_ref),
                _ => {}
            }
            if clear_cross_links
                && matches!(
                    new_state,
                    SequenceState::Terminated
                        | SequenceState::Interrupted
                        | SequenceState::Impossible
                )
            {
                self.clear_cross_postponed_links_to((seq_id, elem_idx));
            }
        }

        // Maintain `actor_in_progress`. The (elem_idx, owner) carried
        // by `entered/left_in_progress` point at whichever element
        // actually transitioned — which can differ from any outer
        // elem_idx the caller passed in (e.g. `stop_element` recurses
        // to a sibling / postponed element).
        if let Some((elem_idx, owner)) = effects.entered_in_progress {
            self.actor_in_progress
                .entry(owner)
                .or_default()
                .insert(SequenceElementRef::new(seq_id, elem_idx));
        }
        if let Some((elem_idx, owner)) = effects.left_in_progress
            && let Some(set) = self.actor_in_progress.get_mut(&owner)
        {
            set.remove(&SequenceElementRef::new(seq_id, elem_idx));
            if set.is_empty() {
                self.actor_in_progress.remove(&owner);
            }
        }

        // A sequence state change calls the completion callback before
        // cascading or calling Ready.  Suspend those trailing effects at
        // exactly that boundary; the engine resumes them after the card's
        // recursive Think has completed.  Impossible is the one exception:
        // the original game starts the postponed sequence element before falling
        // through to the Interrupted/card branch.
        if let Some(mut card) = effects.condolation.take() {
            // If this sequence tear-down came from an in-flight
            // halt, mark the notification so the removal callback
            // handler knows to skip the Think dispatch.
            if self.halt_pending {
                card.from_halt = true;
            }

            card.postponed_successor_pending = card.terminal_state != SequenceState::Impossible
                && effects.resume_cross_postponed.is_some();

            if card.terminal_state == SequenceState::Impossible {
                self.resume_postponed_effects(
                    seq_id,
                    effects.start_postponed.take(),
                    effects.resume_cross_postponed.take(),
                );
            }

            self.pending_condolations.push(PendingCondolationDispatch {
                card,
                effects_after_card: effects,
            });
            return;
        }

        self.process_effects_after_condolation_with_cross_cleanup(
            seq_id,
            effects,
            clear_cross_links,
        );
    }

    pub(super) fn process_effects_after_condolation(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
    ) {
        self.process_effects_after_condolation_with_cross_cleanup(seq_id, effects, true);
    }

    pub(super) fn process_effects_after_condolation_with_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        clear_cross_links: bool,
    ) {
        let install_cross_postponed_after_card = effects.install_cross_postponed_after_card;
        // Process cascading state changes
        for (cascade_elem_idx, cascade_state, cascade_flags) in effects.cascade {
            let sub_effects = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
                    continue;
                };
                if cascade_elem_idx >= seq.elements.len() {
                    continue;
                }
                seq.set_element_state(cascade_elem_idx, cascade_state, cascade_flags)
            };
            // Recursively process sub-effects
            self.process_effects_with_cross_cleanup(
                seq_id,
                sub_effects,
                "terminal_state_cascade",
                clear_cross_links,
            );
        }

        // Signal ready (element finished) — advance to next level
        if effects.signal_ready {
            let to_go = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
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
            self.register_level_elements_to_go(seq_id, to_go);
        }

        // Start postponed element if requested.  We always re-pathfind
        // on restart:
        //
        //   1. Path rebuild: every re-registered Move/Seek element gets
        //      a fresh `InstructOwner` → `try_dispatch_move_path` pass,
        //      and `build_orders_from_path` clears the old orders before
        //      rebuilding waypoints from the actor's current position.
        //   2. We never reassign an element's `command` to
        //      `Command::MoveOk` (see `engine/posture_transitions.rs:281`
        //      for the rationale — flipping to `MoveOk` breaks
        //      `element_priority::actor_branch` priority resolution).
        //      So no element is ever in a `MoveOk` state that would
        //      need a posture-aware revert; the branch is moot.
        self.resume_postponed_effects(
            seq_id,
            effects.start_postponed,
            effects.resume_cross_postponed,
        );

        if let Some((blocker_seq, blocker_idx, waiter_seq, waiter_idx)) =
            install_cross_postponed_after_card
        {
            let waiter_state = self
                .get_element(waiter_seq, waiter_idx)
                .map(|element| element.state)
                .unwrap_or_else(|| {
                    panic!(
                        "deferred cross-postponed waiter {waiter_seq:?}/{waiter_idx} disappeared"
                    )
                });
            assert_eq!(
                waiter_state,
                SequenceState::Postponed,
                "deferred cross-postponed waiter {waiter_seq:?}/{waiter_idx} is not postponed"
            );
            let blocker = self
                .get_element(blocker_seq, blocker_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "deferred cross-postponed blocker {blocker_seq:?}/{blocker_idx} disappeared"
                    )
                });
            assert!(
                blocker.cross_postponed.is_none(),
                "deferred cross-postponed blocker {blocker_seq:?}/{blocker_idx} acquired another waiter"
            );
            self.set_cross_postponed_link(
                (blocker_seq, blocker_idx),
                Some((waiter_seq, waiter_idx)),
            );
        }
    }

    pub(super) fn resume_postponed_effects(
        &mut self,
        seq_id: SequenceId,
        start_postponed: Option<(usize, usize)>,
        resume_cross_postponed: Option<(SequenceId, usize)>,
    ) {
        if let Some((blocker_idx, postponed_idx)) = start_postponed {
            self.register_element_to_go(seq_id, postponed_idx);
            let blocker = self
                .sequences
                .get_mut(&seq_id)
                .and_then(|sequence| sequence.elements.get_mut(blocker_idx))
                .unwrap_or_else(|| {
                    panic!("released postponed blocker {seq_id:?}/{blocker_idx} disappeared")
                });
            assert_eq!(
                blocker.postponed_element_index,
                Some(postponed_idx),
                "released postponed blocker {seq_id:?}/{blocker_idx} no longer points to {postponed_idx}"
            );
            // The original game clears postponed sequence elements
            // `mpsqePostponedSequenceElement` immediately after registering
            // the released element.  Leaving this edge live lets a later
            // same-frame Stop recurse through the stale blocker and interrupt
            // work that is already back on the manager FIFO.
            blocker.postponed_element_index = None;
        }

        // Release the cross-sequence postponed successor. The manager's later
        // restarting calls actor instruction again, which snapshots the
        // actor's posture and action state as they exist at that second
        // instruction boundary. Mark the old snapshot stale so the engine
        // performs the same refresh before arbitration and translation.
        if let Some((succ_seq_id, succ_idx)) = resume_cross_postponed
            && let Some(succ_seq) = self.sequences.get_mut(&succ_seq_id)
            && let Some(succ_elem) = succ_seq.elements.get_mut(succ_idx)
            && succ_elem.state == SequenceState::Postponed
        {
            succ_elem.state = SequenceState::Todo;
            succ_elem.posture_after_transition = crate::element::Posture::Undefined;
            self.register_element_to_go(succ_seq_id, succ_idx);

            // A door route can expose two zero-frame position frontiers at
            // once: Ready registers the old route's AssertPosition, while
            // Postponed-element startup releases a replacement route's
            // AssertPosition. Each assertion immediately makes its following
            // Move ready. Leaving the assertions in old-then-replacement
            // order consequently leaves the replacement Move last, although
            // Original's nested owner boundary has already admitted that
            // replacement before it resumes the old route and lets the old
            // terminal Move reclaim the actor.
            //
            // Scope the correction to that exact same-owner
            // AssertPosition -> Move frontier. A direct PassDoor -> Move
            // successor retains the ordinary Ready-before-postponed FIFO
            // ordering used by equal-priority replacement arbitration.
            if let Some(ready_position) =
                self.cross_postponed_assert_move_frontier(seq_id, succ_seq_id, succ_idx)
            {
                let target = (succ_seq_id, succ_idx);
                let registered_position = self
                    .elements_to_go
                    .iter()
                    .position(|entry| *entry == target)
                    .expect("released cross-postponed assertion was not registered");
                let registered = self
                    .elements_to_go
                    .remove(registered_position)
                    .expect("released cross-postponed assertion disappeared");
                self.elements_to_go.insert(ready_position, registered);
            }
        }
    }

    /// Locate the old route's queued assertion when both the old and released
    /// routes have an immediate `AssertPosition -> Move` frontier.
    pub(super) fn cross_postponed_assert_move_frontier(
        &self,
        ready_sequence_id: SequenceId,
        released_sequence_id: SequenceId,
        released_index: usize,
    ) -> Option<usize> {
        let released_sequence = self.get_sequence(released_sequence_id)?;
        if released_index != 0 || released_sequence.elements.len() != 2 {
            // The f693 handoff is the complete two-element replacement
            // `AssertPosition -> Move`. A routed replacement has further
            // door/assertion work and Original leaves the ordinary
            // Ready-before-postponed FIFO intact; reordering that shape lets
            // the old route's Move interrupt the already-admitted route.
            return None;
        }
        let released = self.get_element(released_sequence_id, released_index)?;
        let released_move = self.get_element(released_sequence_id, released_index + 1)?;
        if released.command != Command::AssertPosition
            || released_move.command != Command::Move
            || released_move.command_level != released.command_level + 1
            || released.owner.is_none()
            || released_move.owner != released.owner
        {
            return None;
        }

        self.elements_to_go.iter().enumerate().rev().find_map(
            |(position, (sequence_id, element_index))| {
                if *sequence_id != ready_sequence_id {
                    return None;
                }
                let ready = self.get_element(*sequence_id, *element_index)?;
                let ready_move = self.get_element(*sequence_id, *element_index + 1)?;
                (ready.command == Command::AssertPosition
                    && ready.state == SequenceState::Todo
                    && ready.owner == released.owner
                    && ready_move.command == Command::Move
                    && ready_move.command_level == ready.command_level + 1
                    && ready_move.owner == ready.owner)
                    .then_some(position)
            },
        )
    }
}
