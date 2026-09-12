//! Sequence manager movement responsibilities.
use super::*;

impl SequenceManager {
    // ─── Termination ────────────────────────────────────────────

    /// Terminate a sequence by interrupting its first element (cascades to all).
    pub fn terminate_sequence(&mut self, seq_id: SequenceId) -> bool {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return false;
        };

        assert!(!seq.is_empty());
        let effects =
            seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::NEXT_LEVEL);
        self.process_effects(seq_id, effects, "terminate_sequence");
        true
    }

    // ─── Cleanup ────────────────────────────────────────────────

    /// Remove completed/interrupted sequences.
    pub fn friday_evening_cleanup(&mut self) {
        self.friday_evening_cleanup_preserving(&std::collections::BTreeSet::new());
    }

    /// Remove completed/interrupted sequences except those still addressed by
    /// an external legacy pointer emulation.
    pub fn friday_evening_cleanup_preserving(
        &mut self,
        retained_sequences: &std::collections::BTreeSet<SequenceId>,
    ) {
        // `BTreeMap::retain` preserves keys, so every `SequenceId`
        // stored elsewhere (`elements_to_go`, `actor_live`,
        // `actor_in_progress`,
        // `cross_postponed`, `post_seek_sequence`, …) stays valid. Any
        // InProgress element in a removed sequence should already be
        // gone via the normal state-transition path, but scrub
        // `actor_in_progress` defensively in case a sequence is torn
        // down without a terminal state change. `elements_to_go`
        // entries for removed ids are dropped lazily by `hourglass`'s
        // existence check.
        let sequence_count_before = self.sequences.len();
        self.sequences
            .retain(|seq_id, seq| retained_sequences.contains(seq_id) || !seq.is_to_be_deleted());
        if self.sequences.len() != sequence_count_before {
            self.postpone_tail_cache.clear();
            self.stop_noop_cache.clear();
        }

        let sequences = &self.sequences;
        self.actor_live.retain(|_, refs| {
            refs.retain(|r| sequences.contains_key(&r.sequence_id));
            !refs.is_empty()
        });
        self.actor_in_progress.retain(|_, refs| {
            refs.retain(|r| sequences.contains_key(&r.sequence_id));
            !refs.is_empty()
        });
    }

    // ─── Cancellation helpers ───────────────────────────────────

    /// Cancel not-yet-launched move commands for a specific actor.
    ///
    /// Walks `elements_to_go` and for every matching entry calls
    /// `set_element_state(Impossible)` *before* removing the element
    /// from the queue. `Impossible` cascades through the next-element
    /// / postponed-element chains and posts a removal notification
    /// to the owner — so successors learn the move became impossible.
    /// (A bare `retain` would drop the queue entries without running
    /// the cascade or queuing the condolation.)
    pub fn cancel_pending_move_commands(&mut self, owner: EntityId) {
        // Pass 1: collect matching `(seq_id, elem_idx)` entries. We
        // can't mutate sequences while iterating `elements_to_go` and
        // we can't mutate `elements_to_go` while iterating `sequences`,
        // so snapshot first.
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner != Some(owner) {
                continue;
            }
            if matches!(
                elem.command,
                Command::PassDoor | Command::Move | Command::WaitTimer | Command::AssertPosition
            ) {
                targets.push((seq_id, elem_idx));
            }
        }

        // Pass 2: mark each target Impossible (cascading next/postponed
        // chains and queuing the owner's condolation card).
        for (seq_id, elem_idx) in &targets {
            let Some(seq) = self.sequences.get_mut(seq_id) else {
                continue;
            };
            let effects = seq.set_element_state(
                *elem_idx,
                SequenceState::Impossible,
                CascadeFlags::NEXT_LEVEL,
            );
            self.process_effects(*seq_id, effects, "cancel_pending_move_commands");
        }

        // Pass 3: drop the cancelled entries from the queue.
        let target_set: std::collections::HashSet<(SequenceId, usize)> =
            targets.into_iter().collect();
        self.elements_to_go
            .retain(|entry| !target_set.contains(entry));
    }

    /// Stop all active and pending sequence elements owned by `owner` whose
    /// priority is weak enough to be pre-empted by `stop_priority`.
    ///
    /// Calls [`Sequence::stop_element`] on the actor's authoritative current
    /// element, then runs [`Self::stop_pending_elements`] for the
    /// not-yet-launched queue. Actor stopping does not scan every live
    /// element owned by the actor: it follows the selected sequence element, whose
    /// postponed chain remains reachable even while a terminating injury's
    /// condolence callback is running. Cross-sequence postponed work is the
    /// Rust representation of that same pointer and is stopped explicitly.
    pub fn stop_owner(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        let root = self.current_element_for_actor(owner);
        self.stop_owner_from_root(owner, root, stop_priority, resolver);
    }

    /// Stop an actor from an explicit root instead of the actor's
    /// currently selected element.
    ///
    /// A command that stops its owner from inside its own translation runs
    /// before the incoming element has been installed as the actor's
    /// selection, yet the original game has already assigned the selected element by
    /// then and therefore stops through the incoming element — reaching
    /// whatever that element pushed into its postponed slot.
    pub fn stop_owner_from_root(
        &mut self,
        owner: EntityId,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        self.stop_owner_current_from_root(owner, root, stop_priority, resolver);
        self.stop_pending_elements(owner, stop_priority, resolver);
    }

    /// Stop only the actor-selected element and its postponed graph.
    ///
    /// Original-game actor stopping has an observable callback boundary:
    /// stopping the selected sequence element synchronously invokes
    /// removal notification, and only after that callback returns does the actor
    /// stop not-yet-launched sequence elements. Engine call sites which can
    /// pump that callback use this phase separately, then call
    /// [`Self::stop_pending_elements`] after the callback has completed.
    pub fn stop_owner_current_from_root(
        &mut self,
        owner: EntityId,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        // Stopping a sequence element always follows the postponed pointer,
        // even when the current node is too strong. That is observable if a
        // weaker descendant exists, but it is a pure no-op when every live
        // element owned by this actor is stronger than `stop_priority`.
        // Checking the actor-wide ceiling is deliberately conservative: it
        // can decline this fast path because of unrelated weak work, but can
        // never hide a stoppable node in the selected graph. Terminal nodes
        // are removed from postponed links at their state-transition cleanup.
        let root_is_live = root.is_some_and(|(seq_id, elem_idx)| {
            self.get_element(seq_id, elem_idx).is_some_and(|element| {
                element.owner == Some(owner)
                    && Self::is_actor_live_state(element.state)
                    && !(element.command == Command::Wait
                        && element.priority == SequencePriority::Wait)
            })
        });
        let selected_chain_is_all_stronger =
            root.is_some_and(|root| self.selected_cross_chain_all_stronger(root, stop_priority));
        let selected_stop_cached_noop =
            root.is_some_and(|root| self.selected_stop_is_cached_noop(root, stop_priority));
        if root_is_live
            && (selected_stop_cached_noop
                || selected_chain_is_all_stronger
                || self.actor_stop_summary(owner).is_some_and(|summary| {
                    summary.cross_only && summary.weakest_priority < stop_priority
                }))
        {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                ?root,
                selected_stop_cached_noop,
                selected_chain_is_all_stronger,
                "manager stop_owner all live work too strong"
            );
            return;
        }

        // Original-game actor stopping starts from exactly
        // selected sequence element. Scanning every InProgress/Postponed element is
        // observably different after loading: stale non-selected branches can
        // form a large shared next/postponed graph, so recursively stopping
        // each node as a fresh root repeats that graph exponentially.
        let mut targets = VecDeque::new();
        if let Some(current) = root
            && self.get_element(current.0, current.1).is_some_and(|elem| {
                !(elem.command == Command::Wait && elem.priority == SequencePriority::Wait)
            })
        {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                ?current,
                "manager stop_owner current"
            );
            targets.push_back(current);
        }
        if targets.is_empty() {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                "manager stop_owner no current target"
            );
        }
        let mut visited = HashSet::new();
        let mut touched_sequences = HashSet::new();
        let mut effect_count = 0usize;
        let mut terminal_transition_count = 0usize;
        while let Some((seq_id, elem_idx)) = targets.pop_front() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }
            let target_owner = self
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("Stop postponed graph references missing {seq_id:?}/{elem_idx}")
                })
                .owner;
            touched_sequences.insert(seq_id);
            assert_eq!(
                target_owner,
                Some(owner),
                "Stop postponed graph crosses owners at {seq_id:?}/{elem_idx}"
            );
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?seq_id,
                elem_idx,
                "manager before stop_element"
            );
            let (effects_vec, cross_targets) = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated Stop target sequence disappeared")
                .stop_element_with_cross_targets(elem_idx, stop_priority, resolver);
            for cross in cross_targets {
                if !visited.contains(&cross) {
                    targets.push_back(cross);
                }
            }
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?seq_id,
                elem_idx,
                effects = effects_vec.len(),
                "manager after stop_element"
            );
            for (effect_index, effects) in effects_vec.into_iter().enumerate() {
                effect_count += 1;
                terminal_transition_count +=
                    usize::from(effects.actor_live_transition.is_some_and(
                        |(_, _, _, new_state)| {
                            matches!(
                                new_state,
                                SequenceState::Terminated
                                    | SequenceState::Interrupted
                                    | SequenceState::Impossible
                            )
                        },
                    ));
                tracing::trace!(
                    target: "parity_stop",
                    ?owner,
                    ?seq_id,
                    elem_idx,
                    effect_index,
                    "manager before process_effects"
                );
                self.process_effects_deferring_cross_cleanup(seq_id, effects, "stop_owner");
                tracing::trace!(
                    target: "parity_stop",
                    ?owner,
                    ?seq_id,
                    elem_idx,
                    effect_index,
                    "manager after process_effects"
                );
            }
        }

        tracing::trace!(
            target: "parity_stop_cache",
            ?owner,
            ?root,
            ?stop_priority,
            effect_count,
            terminal_transition_count,
            touched_sequence_count = touched_sequences.len(),
            "completed selected Stop traversal"
        );
        if terminal_transition_count == 0 {
            if let Some(root) = root {
                self.repair_selected_stop_noop(owner, root, stop_priority);
            }
            return;
        }

        // The original game clears postponed references while this recursive stop graph
        // unwinds. Restrict Rust's split-storage cleanup to the sequences the
        // graph actually visited: EnterSwordfight can invoke Stop thousands of
        // times in one frame, so even one retained-manager scan per invocation
        // is quadratic in replay history.
        self.clear_terminal_cross_postponed_links_in(&touched_sequences);
    }

    /// Drop blocker links whose postponed target was stopped either directly
    /// or by a following-element cascade.
    ///
    /// Original-game sequence stopping recursively stops its postponed
    /// element and nulls the pointer when that target becomes interrupted.
    /// Rust can reach the same target through `CASCADE_FOLLOWING`, in which
    /// case it is absent from the direct `stopped` list above. Retaining that
    /// link past Friday cleanup would leave a dangling sequence reference.
    pub(super) fn clear_terminal_cross_postponed_links(&mut self) {
        let dead_targets: std::collections::HashSet<(SequenceId, usize)> =
            self.sequences
                .iter()
                .flat_map(|(sequence_id, sequence)| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            matches!(
                                element.state,
                                SequenceState::Terminated
                                    | SequenceState::Interrupted
                                    | SequenceState::Impossible
                            )
                            .then_some((*sequence_id, element_index))
                        },
                    )
                })
                .collect();
        let mut changed_owners = BTreeSet::new();
        for sequence in self.sequences.values_mut() {
            for element in &mut sequence.elements {
                if element
                    .cross_postponed
                    .is_some_and(|target| dead_targets.contains(&target))
                {
                    if let Some(owner) = element.owner {
                        changed_owners.insert(owner);
                    }
                    element.cross_postponed = None;
                }
            }
        }
        for owner in changed_owners {
            self.invalidate_postpone_tail_cache_for(owner);
        }
    }

    /// Clear dead cross-postponed successors only from sequences visited by a
    /// bounded Stop graph. This is the local equivalent of Original nulling a
    /// postponed pointer while that recursive call unwinds.
    pub(super) fn clear_terminal_cross_postponed_links_in(
        &mut self,
        source_sequences: &HashSet<SequenceId>,
    ) {
        let mut candidate_links = Vec::new();
        for sequence_id in source_sequences {
            let Some(sequence) = self.sequences.get(sequence_id) else {
                continue;
            };
            for (element_index, element) in sequence.elements.iter().enumerate() {
                if let Some(target) = element.cross_postponed {
                    candidate_links.push((*sequence_id, element_index, target));
                }
            }
        }

        let dead_sources = candidate_links
            .into_iter()
            .filter_map(|(sequence_id, element_index, target)| {
                self.get_element(target.0, target.1)
                    .is_some_and(|target_element| {
                        matches!(
                            target_element.state,
                            SequenceState::Terminated
                                | SequenceState::Interrupted
                                | SequenceState::Impossible
                        )
                    })
                    .then_some((sequence_id, element_index))
            })
            .collect::<Vec<_>>();

        for (sequence_id, element_index) in dead_sources {
            self.set_cross_postponed_link((sequence_id, element_index), None);
        }
    }

    pub(super) fn clear_cross_postponed_links_to(&mut self, target: (SequenceId, usize)) {
        let mut changed_owners = BTreeSet::new();
        for sequence in self.sequences.values_mut() {
            for element in &mut sequence.elements {
                if element.cross_postponed == Some(target) {
                    if let Some(owner) = element.owner {
                        changed_owners.insert(owner);
                    }
                    element.cross_postponed = None;
                }
            }
        }
        for owner in changed_owners {
            self.invalidate_postpone_tail_cache_for(owner);
        }
    }

    /// Stop not-yet-launched elements for a specific actor up to a priority.
    pub fn stop_pending_elements(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        // Work from the same roots as Original's
        // Stop not-yet-launched sequence elements: entries currently registered in
        // the manager's to-go list for this owner. A root can be too strong to
        // stop while still owning a postponed pointer; sequence stopping
        // follows that pointer unconditionally, so retain the cross-sequence
        // targets returned by Rust's split-storage representation.
        let roots = self.pending_elements_for_owner(owner);
        self.stop_pending_roots(owner, roots, stop_priority, resolver);
        self.compact_terminal_elements_to_go();
    }

    /// Snapshot the entries which the original game will visit when stopping
    /// unlaunched elements. The loop captures
    /// the list size on entry, so callback-appended work is deliberately not
    /// part of this result.
    pub fn pending_elements_for_owner(&self, owner: EntityId) -> Vec<(SequenceId, usize)> {
        self.elements_to_go
            .iter()
            .copied()
            .filter(|(seq_id, elem_idx)| {
                self.get_element(*seq_id, *elem_idx).is_some_and(|element| {
                    element.owner == Some(owner) && element.state != SequenceState::Interrupted
                })
            })
            .collect()
    }

    /// Physically remove terminal tombstones after a callback-separated
    /// pending Stop scan. State-aware registration queries hide each stopped
    /// entry immediately; compacting once after the snapshot finishes keeps
    /// the stable manager queue identical to Original without an O(queue)
    /// retain after every root.
    pub(crate) fn compact_terminal_elements_to_go(&mut self) {
        self.elements_to_go.retain(|(seq_id, elem_idx)| {
            self.sequences.get(seq_id).is_none_or(|sequence| {
                sequence
                    .elements
                    .get(*elem_idx)
                    .is_none_or(|element| element.state != SequenceState::Interrupted)
            })
        });
    }

    /// Stop one root from a previously captured pending-list snapshot.
    /// Callers that model actor stopping can close the resulting synchronous
    /// condolence stack before visiting the next captured root.
    pub fn stop_pending_element_from_root(
        &mut self,
        owner: EntityId,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        self.stop_pending_roots(owner, [root], stop_priority, resolver);
    }

    pub(super) fn stop_pending_roots(
        &mut self,
        owner: EntityId,
        roots: impl IntoIterator<Item = (SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        let mut targets = roots.into_iter().collect::<VecDeque<_>>();
        let mut visited = HashSet::new();
        let mut touched_sequences = HashSet::new();

        while let Some((seq_id, elem_idx)) = targets.pop_front() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            touched_sequences.insert(seq_id);
            assert_eq!(
                seq.elements[elem_idx].owner,
                Some(owner),
                "pending postponed graph crosses owners at {seq_id:?}/{elem_idx}"
            );

            let (effects_vec, cross_targets) = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated pending sequence disappeared")
                .stop_element_with_cross_targets(elem_idx, stop_priority, resolver);
            for target in cross_targets {
                if !visited.contains(&target) {
                    targets.push_back(target);
                }
            }
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_pending_elements",
                );
            }
        }

        // This API is called once per captured pending root so the engine can
        // dispatch that root's synchronous condolence before advancing the
        // snapshot. A global retained-sequence cleanup here makes the whole
        // scan quadratic. Every cross edge followed by this bounded Stop has
        // its source in a touched sequence, so restrict cleanup to that set.
        self.clear_terminal_cross_postponed_links_in(&touched_sequences);
    }

    /// Stop queued elements for `owner` whose command matches `command`.
    /// Counterpart to [`Self::stop_pending_elements`] with a command
    /// filter — used by the right-click `Bow` arm to drain the PC's
    /// queued `Command::ShootBow` elements without cancelling other
    /// in-flight work.
    ///
    /// This covers both not-yet-launched `elements_to_go` entries and
    /// cross-postponed elements. The original game stores repeated PC bow clicks in
    /// `mlpsequenceShootList`; in Rust those clicks may be represented
    /// as `SequenceState::Postponed`, so clearing the queue must see
    /// both forms.
    ///
    /// Returns the number of pending elements that were stopped + removed.
    pub fn stop_pending_elements_matching(
        &mut self,
        owner: EntityId,
        command: Command,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) -> usize {
        let has_matching_live_element = self.actor_live.get(&owner).is_some_and(|refs| {
            refs.iter().any(|element_ref| {
                self.get_element(element_ref.sequence_id, element_ref.element_index)
                    .is_some_and(|element| {
                        element.command == command && element.state != SequenceState::InProgress
                    })
            })
        });
        if !has_matching_live_element {
            return 0;
        }

        let mut to_remove = Vec::new();
        let mut terminal_effects_processed = false;

        for i in 0..self.elements_to_go.len() {
            let (seq_id, elem_idx) = self.elements_to_go[i];
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner != Some(owner) || elem.command != command {
                continue;
            }
            if elem.state == SequenceState::InProgress {
                continue;
            }

            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated pending sequence disappeared")
                .stop_element(elem_idx, stop_priority, resolver);
            terminal_effects_processed |= !effects_vec.is_empty();
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_pending_elements_matching",
                );
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                to_remove.push(i);
            }
        }

        let count = to_remove.len();
        for &idx in to_remove.iter().rev() {
            self.elements_to_go.remove(idx);
        }

        // `actor_live` contains every Todo/InProgress/Postponed element for
        // this owner. Use it to avoid a complete retained-sequence scan for
        // the overwhelmingly common no-match case (for example each queued
        // EnterSwordfight checking for an old ShootBow). Restore manager
        // insertion order before dispatch so loaded non-monotonic sequence IDs
        // preserve Original's first-to-last traversal.
        let mut postponed_targets = self
            .actor_live
            .get(&owner)
            .into_iter()
            .flat_map(|refs| refs.iter())
            .filter_map(|element_ref| {
                self.get_element(element_ref.sequence_id, element_ref.element_index)
                    .is_some_and(|element| {
                        element.command == command && element.state == SequenceState::Postponed
                    })
                    .then_some((element_ref.sequence_id, element_ref.element_index))
            })
            .collect::<Vec<_>>();
        postponed_targets.sort_by_key(|(sequence_id, element_index)| {
            (
                self.sequences
                    .get_index_of(sequence_id)
                    .unwrap_or_else(|| panic!("actor-live sequence {sequence_id:?} disappeared")),
                *element_index,
            )
        });

        let mut stopped_count = count;
        for (seq_id, elem_idx) in postponed_targets {
            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .expect("collected postponed sequence disappeared")
                .stop_element(elem_idx, stop_priority, resolver);
            terminal_effects_processed |= !effects_vec.is_empty();
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_postponed_elements_matching",
                );
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                stopped_count += 1;
            }
        }

        if terminal_effects_processed {
            // As in `stop_owner_current_from_root`, clear inbound links once
            // for the completed batch instead of rescanning the complete
            // manager for every matching pending or postponed element.
            self.clear_terminal_cross_postponed_links();
        }

        stopped_count
    }

    /// Returns `true` if `owner` has a queued element with this command.
    /// Includes both not-yet-launched `elements_to_go` entries and
    /// cross-postponed elements.
    pub fn queued_element_exists(&self, owner: EntityId, command: Command) -> bool {
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            let Some(elem) = seq.elements.get(elem_idx) else {
                continue;
            };
            if elem.owner == Some(owner)
                && elem.command == command
                && elem.state != SequenceState::InProgress
            {
                return true;
            }
        }
        self.sequences.values().any(|seq| {
            seq.elements.iter().any(|elem| {
                elem.owner == Some(owner)
                    && elem.command == command
                    && elem.state == SequenceState::Postponed
            })
        })
    }

    /// Check if there's a pending element with this command for this owner.
    pub fn element_is_about_to_be_launched(&self, owner: EntityId, command: Command) -> bool {
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner == Some(owner) && (command == Command::Null || elem.command == command) {
                return true;
            }
        }
        false
    }

    /// Check the two pending-command forms used by Original's actor AI:
    /// an element registered to launch, or an element postponed directly
    /// behind the actor's current element.
    ///
    /// This deliberately does not scan every postponed element owned by the
    /// actor. The original game follows the actor's sequence element to its postponed element
    /// here, so only the current element's immediate successor qualifies.
    pub fn element_is_about_to_be_launched_or_postponed_by_current(
        &self,
        owner: EntityId,
        command: Command,
    ) -> bool {
        if self.element_is_about_to_be_launched(owner, command) {
            return true;
        }

        let Some((seq_id, elem_idx)) = self.current_element_for_actor(owner) else {
            return false;
        };
        let Some(current) = self.get_element(seq_id, elem_idx) else {
            debug_assert!(false, "current actor element is missing from its sequence");
            return false;
        };

        let intra_sequence_matches = current
            .postponed_element_index
            .and_then(|postponed_idx| self.get_element(seq_id, postponed_idx))
            .is_some_and(|postponed| postponed.command == command);
        let cross_sequence_matches = current
            .cross_postponed
            .and_then(|(postponed_seq, postponed_idx)| {
                self.get_element(postponed_seq, postponed_idx)
            })
            .is_some_and(|postponed| postponed.command == command);

        intra_sequence_matches || cross_sequence_matches
    }

    /// Apply fast-movement conversion to all active/pending movement elements owned by
    /// `entity` in its selected element's following/postponed chain. Sets the
    /// FAST flag, upgrades the element's action from walking to running, and
    /// rewrites queued walking / start-walking / stop-walking orders.
    pub fn make_fast(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_fast_element);
    }

    /// Set `action` on the movement element at `(seq_id, elem_idx)` and recurse
    /// through its same-owner following/postponed graph. Callers use this to
    /// force a door-authored movement chain onto one animation.
    pub fn set_action_recursive(&mut self, seq_id: SequenceId, elem_idx: usize, action: OrderType) {
        let Some(root) = self.get_element(seq_id, elem_idx) else {
            return;
        };
        let owner = root.owner;
        let mut visited = HashSet::new();
        let mut pending = vec![(seq_id, elem_idx)];
        while let Some((sid, idx)) = pending.pop() {
            if !visited.insert((sid, idx)) {
                continue;
            }
            let Some(element) = self.get_element(sid, idx) else {
                continue;
            };
            if element.owner != owner {
                continue;
            }
            let following = self.rewrite_following_ref(sid, idx);
            let postponed = element
                .cross_postponed
                .or_else(|| element.postponed_element_index.map(|next| (sid, next)));

            self.get_element_mut(sid, idx)
                .expect("recursive action-assignment graph element disappeared")
                .set_action(action);
            if let Some(following) = following {
                pending.push(following);
            }
            if let Some(postponed) = postponed {
                pending.push(postponed);
            }
        }
    }

    /// Apply slow-movement conversion to all active/pending movement elements owned by
    /// `entity`. Clears the FAST flag, downgrades running animations to
    /// walking, and rewrites queued transition orders accordingly.
    ///
    /// Symmetric counterpart to [`Self::make_fast`].
    pub fn make_slow(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_slow_element);
    }

    /// Apply upright-posture conversion to all active/pending elements owned by
    /// `entity`. Rewrites crouched-movement orders to upright variants
    /// and cancels pending `CrouchDown` sequence elements (their
    /// command is demoted to `Null`).
    pub fn make_upright(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_upright_element);
    }

    /// Apply crouched-posture conversion to all active/pending elements owned by
    /// `entity`. Clears the FAST flag, downgrades running/walking
    /// upright orders to crouched, and rewrites posture-transition
    /// orders accordingly.
    pub fn make_crouched(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_crouched_element);
    }

    /// Reproduce selected-element transition construction: start at the actor's selected
    /// element and recurse only through same-owner `mpsqeNextSequenceElement`
    /// and `mpsqePostponedSequenceElement` links. An unrelated queued sequence
    /// owned by the same actor is not part of that graph and must not change.
    pub(super) fn rewrite_selected_actor_chain(
        &mut self,
        entity: EntityId,
        rewrite: fn(&mut SequenceElement),
    ) {
        let Some(root) = self.current_element_for_actor(entity) else {
            return;
        };
        let mut pending = vec![root];
        let mut visited = HashSet::new();

        while let Some((seq_id, elem_idx)) = pending.pop() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }

            let Some(element) = self.get_element(seq_id, elem_idx) else {
                continue;
            };
            if element.owner != Some(entity) {
                continue;
            }

            let following = self.rewrite_following_ref(seq_id, elem_idx);
            let postponed = element
                .cross_postponed
                .or_else(|| element.postponed_element_index.map(|idx| (seq_id, idx)));

            rewrite(
                self.get_element_mut(seq_id, elem_idx)
                    .expect("selected Make* chain element disappeared"),
            );

            if let Some(next) = following {
                pending.push(next);
            }
            if let Some(postponed) = postponed {
                pending.push(postponed);
            }
        }
    }

    /// Find the next movement/jump element owned by the same entity, in
    /// either this element's own sequence (following cursor) or in the
    /// attached `post_seek_sequence` if any.
    ///
    /// Returns `true` if the next element (owned by the same entity) is
    /// itself a movement element; `false` if there is no such element
    /// or the owner differs.
    pub fn is_next_movement(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.next_element_in_chain(seq_id, elem_idx)
            .and_then(|(s, i)| self.get_element(s, i))
            .map(|next| next.data.is_movement())
            .unwrap_or(false)
    }

    /// As [`Self::is_next_movement`], but also accepts `Command::JumpCmd`.
    pub fn is_next_movement_or_jump(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.next_element_in_chain(seq_id, elem_idx)
            .and_then(|(s, i)| self.get_element(s, i))
            .map(|next| next.data.is_movement() || next.command == Command::JumpCmd)
            .unwrap_or(false)
    }

    /// Stop the currently-executing movement order for `entity` and
    /// cancel any in-flight path request. Returns `true` if at least
    /// one element was rewritten or had its path cancelled.
    ///
    /// `owner_pos` is the owner's current map position (used to shorten
    /// the movement destination to ~10 units ahead); `cancel_path` is
    /// invoked when any element in the `MoveWaiting` state needs its
    /// pending path request dropped.
    ///
    /// `stop_priority` gates the rewrite: it only runs when the
    /// element's priority is `>= stop_priority` (weaker or equal).
    /// `resolver` lazily promotes `NotYetSet` priorities (mirroring
    /// `Sequence::stop_element`).
    pub fn stop_movement_for_owner(
        &mut self,
        entity: EntityId,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            entity,
            None,
            owner_pos,
            stop_priority,
            resolver,
            next_order_id,
            cancel_path,
        )
    }

    /// Run the movement-specific stopping phase for exactly one
    /// selected element. This is the narrow counterpart to
    /// [`Self::stop_movement_for_owner`]: call sites modeling
    /// Stopping the selected element must not rewrite unrelated in-progress
    /// movements owned by the same actor.
    pub fn stop_movement_from_root(
        &mut self,
        entity: EntityId,
        root: (SequenceId, usize),
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            entity,
            Some(root),
            owner_pos,
            stop_priority,
            resolver,
            next_order_id,
            cancel_path,
        )
    }

    pub(super) fn stop_movement_for_owner_from_root(
        &mut self,
        entity: EntityId,
        root: Option<(SequenceId, usize)>,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        let mut changed = false;
        let mut to_interrupt: Vec<(SequenceId, usize)> = Vec::new();
        let refs: Vec<SequenceElementRef> = if let Some((sequence_id, element_index)) = root {
            vec![SequenceElementRef {
                sequence_id,
                element_index,
            }]
        } else {
            let Some(refs) = self.actor_in_progress.get(&entity) else {
                return false;
            };
            refs.iter().copied().collect()
        };
        for elem_ref in refs {
            let Some(seq) = self.sequences.get(&elem_ref.sequence_id) else {
                debug_assert!(false, "actor_in_progress contains stale sequence ref");
                continue;
            };
            let seq_id = seq.id;
            let elem_idx = elem_ref.element_index;
            let Some(elem) = seq.elements.get(elem_idx) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if elem.owner != Some(entity)
                || elem.state != SequenceState::InProgress
                || !elem.data.is_movement()
            {
                continue;
            }
            // Without this priority gate, a weaker `Preference`-
            // priority stop would still rewrite the order of a
            // stronger `Script`-priority movement, causing a visual
            // stutter even though `SequenceManager::stop_owner` will
            // then refuse to actually interrupt the element.
            if self.resolve_element_stop_priority(seq_id, elem_idx, resolver) < stop_priority {
                continue;
            }
            let elem = self
                .get_element_mut(seq_id, elem_idx)
                .expect("selected movement disappeared after priority resolution");
            // Clear SEEK bit; rewrite first order's animation to the
            // matching waiting-transition variant.
            if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
                *flags &= !MoveFlags::SEEK;
            }
            let Some(first) = elem.orders.front_mut() else {
                continue;
            };
            let new_action = match first.order_type {
                crate::order::OrderType::WalkingUpright => {
                    Some(crate::order::OrderType::TransitionWalkingUprightWaitingUpright)
                }
                crate::order::OrderType::RunningUpright => {
                    Some(crate::order::OrderType::TransitionRunningUprightWaitingUpright)
                }
                crate::order::OrderType::WalkingCrouched => {
                    Some(crate::order::OrderType::TransitionWalkingCrouchedWaitingCrouched)
                }
                _ => None,
            };
            let Some(action) = new_action else {
                // Default case: no matching transition — the whole
                // element must be interrupted.  Path cancellation
                // fires on the `Interrupted` transition, so we
                // schedule the state change and run the cascade +
                // path cancellation together below.
                to_interrupt.push((seq_id, elem_idx));
                continue;
            };
            first.order_type = action;
            // Bumping the order id forces the actor-tick consumer
            // (`last_order_id != order.unique_id`) to retrigger
            // `new_order`, which the sprite pipeline uses to reset
            // `MotionState::Start` + `initialize_action_done` so the
            // rewritten Transition*Waiting* animation plays from the
            // first frame.
            first.reseed_id(crate::order::alloc_order_id(next_order_id));
            changed = true;
            // Trim trailing orders and shorten the movement element's
            // destination to ~10 units along the current heading.
            //
            // The original game's movement stop resets the destination, whose inline
            // setter changes only the destination point; it deliberately does not
            // rewrite the order's 2D destination. The transition order
            // therefore reinitializes the sprite against its old path goal,
            // while the element retains the shortened logical destination.
            elem.orders.truncate(1);
            let first = elem.orders.front().expect("truncate kept 1 order");
            let vx = first.target_x - owner_pos.x;
            let vy = first.target_y - owner_pos.y;
            let norm = (vx * vx + vy * vy).sqrt();
            if norm > 10.0 {
                let scale = 10.0 / norm;
                let new_x = owner_pos.x + vx * scale;
                let new_y = owner_pos.y + vy * scale;
                if let SequenceElementData::Movement { destination, .. } = &mut elem.data {
                    destination.x = new_x;
                    destination.y = new_y;
                }
            }
        }
        // Only fire path cancellation for elements that actually
        // transitioned to INTERRUPTED.  A successful rewrite leaves
        // the element in INPROGRESS and keeps the path request alive.
        for (seq_id, elem_idx) in to_interrupt {
            let effects = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
                    continue;
                };
                if seq.elements[elem_idx].command == Command::MoveWaiting {
                    seq.elements[elem_idx].command = Command::Move;
                    cancel_path(entity);
                }
                seq.set_element_state(
                    elem_idx,
                    SequenceState::Interrupted,
                    CascadeFlags::NEXT_LEVEL,
                )
            };
            self.process_effects(seq_id, effects, "interrupt_move_towards");
            changed = true;
        }
        changed
    }

    /// Resolve "next element in chain" for
    /// [`Self::is_next_movement`]/`is_next_movement_or_jump`. Follows the
    /// exact loaded v48 following pointer when present; runtime-authored
    /// sequences use append order. The next owner must match.
    ///
    /// Post-seek sequences are stored as separate `Sequence`s registered with
    /// the manager; they are not an implicit following edge. A loaded
    /// `mpsqeNextSequenceElement`, however, is authoritative even when null or
    /// non-adjacent.
    pub(super) fn next_element_in_chain(
        &self,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Option<(SequenceId, usize)> {
        let this = self.get_element(seq_id, elem_idx)?;
        let (next_seq, next_idx) = self.unsevered_following_ref(seq_id, elem_idx)?;
        let next = self.get_element(next_seq, next_idx)?;
        if this.owner == next.owner {
            Some((next_seq, next_idx))
        } else {
            None
        }
    }

    /// Returns `true` when no further "real" sequence element follows
    /// this one — i.e. the sequence is effectively done after this
    /// element finishes.  `Wait` and `AssertPosition` are skipped
    /// (treated as non-actions).
    pub fn is_last_real_action(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        let mut cur = (seq_id, elem_idx);
        loop {
            // The original game recursively checks the last real action for every skipped
            // Wait/AssertPosition, and each invocation checks that node's
            // postponed pointer before following `next` again.
            let Some(this) = self.get_element(cur.0, cur.1) else {
                return true;
            };
            if this.postponed_element_index.is_some() || this.cross_postponed.is_some() {
                return false;
            }
            // The last-real-action check follows the raw
            // next-element link without requiring the next element
            // to have the same owner. This differs deliberately from the
            // movement-chain queries above: a manager-owned Timer or an
            // action for another actor still suppresses this actor's
            // condolence callback when it follows in the same sequence.
            //
            // Preserve the original game's NPC sequence cleanup behavior.
            let Some((next_seq, next_idx)) = self.unsevered_following_ref(cur.0, cur.1) else {
                return true;
            };
            let Some(next_elem) = self.get_element(next_seq, next_idx) else {
                return true;
            };
            match next_elem.command {
                Command::Wait | Command::AssertPosition => {
                    cur = (next_seq, next_idx);
                    continue;
                }
                _ => return false,
            }
        }
    }
}
