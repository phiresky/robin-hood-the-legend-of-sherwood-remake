//! Sequence manager movement responsibilities.
use super::*;

impl SequenceManager {
    // ─── Termination ────────────────────────────────────────────

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
        let deleted: BTreeSet<_> = self
            .sequences
            .iter()
            .filter_map(|(id, sequence)| {
                (!retained_sequences.contains(id) && sequence.is_to_be_deleted()).then_some(*id)
            })
            .collect();
        if !deleted.is_empty() {
            // Clear incoming pointers before destroying their target elements.
            // Terminal state alone does not sever a link: retained sequences
            // remain addressable until their actual deletion boundary.
            for (id, sequence) in self.sequences.iter_mut() {
                if deleted.contains(id) {
                    continue;
                }
                for element in &mut sequence.elements {
                    if element
                        .next
                        .is_some_and(|next| deleted.contains(&next.sequence_id))
                    {
                        element.next = None;
                    }
                    if element
                        .postponed
                        .is_some_and(|next| deleted.contains(&next.sequence_id))
                    {
                        element.postponed = None;
                    }
                }
            }
            self.sequences.retain(|id, _| !deleted.contains(id));
            self.postpone_tail_cache.clear();
        }

        let sequences = &self.sequences;
        self.actor_live.retain(|_, refs| {
            refs.retain(|r| sequences.contains_key(&r.sequence_id));
            !refs.is_empty()
        });
    }

    // ─── Cancellation helpers ───────────────────────────────────

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
        entities: &crate::entities::Entities,
        owner: EntityId,
        command: Command,
    ) -> bool {
        if self.element_is_about_to_be_launched(owner, command) {
            return true;
        }

        let Some((seq_id, elem_idx)) = entities.current_element_for_actor(owner) else {
            return false;
        };
        let Some(current) = self.get_element(seq_id, elem_idx) else {
            debug_assert!(false, "current actor element is missing from its sequence");
            return false;
        };

        current
            .postponed
            .and_then(|reference| self.get_element(reference.sequence_id, reference.element_index))
            .is_some_and(|postponed| postponed.command == command)
    }

    /// Apply fast-movement conversion to all active/pending movement elements owned by
    /// `entity` in its selected element's following/postponed chain. Sets the
    /// FAST flag, upgrades the element's action from walking to running, and
    /// rewrites queued walking / start-walking / stop-walking orders.
    pub fn make_fast(&mut self, entities: &crate::entities::Entities, entity: EntityId) {
        self.rewrite_selected_actor_chain(entities, entity, make_fast_element);
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
                .postponed
                .map(|reference| (reference.sequence_id, reference.element_index));

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
    pub fn make_slow(&mut self, entities: &crate::entities::Entities, entity: EntityId) {
        self.rewrite_selected_actor_chain(entities, entity, make_slow_element);
    }

    /// Apply upright-posture conversion to all active/pending elements owned by
    /// `entity`. Rewrites crouched-movement orders to upright variants
    /// and cancels pending `CrouchDown` sequence elements (their
    /// command is demoted to `Null`).
    pub fn make_upright(&mut self, entities: &crate::entities::Entities, entity: EntityId) {
        self.rewrite_selected_actor_chain(entities, entity, make_upright_element);
    }

    /// Apply crouched-posture conversion to all active/pending elements owned by
    /// `entity`. Clears the FAST flag, downgrades running/walking
    /// upright orders to crouched, and rewrites posture-transition
    /// orders accordingly.
    pub fn make_crouched(&mut self, entities: &crate::entities::Entities, entity: EntityId) {
        self.rewrite_selected_actor_chain(entities, entity, make_crouched_element);
    }

    /// Reproduce selected-element transition construction: start at the actor's selected
    /// element and recurse only through same-owner `mpsqeNextSequenceElement`
    /// and `mpsqePostponedSequenceElement` links. An unrelated queued sequence
    /// owned by the same actor is not part of that graph and must not change.
    pub(super) fn rewrite_selected_actor_chain(
        &mut self,
        entities: &crate::entities::Entities,
        entity: EntityId,
        rewrite: fn(&mut SequenceElement),
    ) {
        let Some(root) = entities.current_element_for_actor(entity) else {
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
                .postponed
                .map(|reference| (reference.sequence_id, reference.element_index));

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
            if this.postponed.is_some() {
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

impl crate::engine::EngineInner {
    fn resolve_live_sequence_stop_priority(
        &mut self,
        reference: SequenceElementRef,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) -> SequencePriority {
        let element = self
            .orders
            .sequence_manager
            .get_element(reference.sequence_id, reference.element_index)
            .expect("stopped element missing");
        if element.priority != SequencePriority::NotYetSet {
            return element.priority;
        }
        let resolved = if element.owner.is_some_and(|owner| {
            self.world
                .entities
                .get(owner)
                .expect("stopped element owner missing")
                .is_actor()
        }) {
            resolver(self, element)
        } else {
            SequencePriority::Normal
        };
        let priority = match resolved {
            SequencePriority::None => SequencePriority::Normal,
            priority => priority,
        };
        self.orders.sequence_manager.set_element_priority(
            reference.sequence_id,
            reference.element_index,
            priority,
        );
        priority
    }

    fn stop_live_sequence_element(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        reference: SequenceElementRef,
        priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
        call_path: &mut HashSet<SequenceElementRef>,
    ) {
        // Only traversal return addresses live here. Every state transition and
        // owner callback completes synchronously before the next link is read.
        // Keeping deep graph traversal off the machine stack avoids one large
        // Engine dispatcher frame per postponed element.
        #[derive(Clone, Copy, Serialize, Deserialize)]
        enum Frame {
            Enter(SequenceElementRef),
            AfterFollowing(SequenceElementRef),
            Postponed(SequenceElementRef),
            AfterPostponed(SequenceElementRef),
        }
        let mut frames = vec![Frame::Enter(reference)];
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Enter(reference) => {
                    assert!(call_path.insert(reference), "cycle in sequence Stop");
                    self.resolve_live_sequence_stop_priority(reference, resolver);
                    let sequence = self
                        .orders
                        .sequence_manager
                        .get_sequence(reference.sequence_id)
                        .expect("stopped sequence missing");
                    let action = sequence.prepare_element_stop(reference.element_index, priority);
                    frames.push(Frame::Postponed(reference));
                    match action {
                        StopElementAction::InterruptSelf => self.element_interrupted(
                            sim,
                            assets,
                            active_scripts,
                            reference.sequence_id,
                            reference.element_index,
                            CascadeFlags::NEXT_LEVEL,
                        ),
                        StopElementAction::InterruptFollowing
                        | StopElementAction::StopFollowing => {
                            if let Some(next) = sequence.live_following_ref(reference.element_index)
                            {
                                if action == StopElementAction::InterruptFollowing {
                                    self.element_interrupted(
                                        sim,
                                        assets,
                                        active_scripts,
                                        next.sequence_id,
                                        next.element_index,
                                        CascadeFlags::NEXT_LEVEL,
                                    );
                                } else {
                                    frames.push(Frame::AfterFollowing(reference));
                                    frames.push(Frame::Enter(next));
                                }
                            }
                        }
                        StopElementAction::NoChange => {}
                    }
                }
                Frame::AfterFollowing(reference) => {
                    let current_next = self
                        .orders
                        .sequence_manager
                        .get_sequence(reference.sequence_id)
                        .expect("stopped sequence missing")
                        .live_following_ref(reference.element_index);
                    if current_next.is_some_and(|next| {
                        self.orders
                            .sequence_manager
                            .get_element(next.sequence_id, next.element_index)
                            .expect("following Stop target missing")
                            .state
                            == SequenceState::Interrupted
                    }) {
                        self.orders
                            .sequence_manager
                            .get_sequence_mut(reference.sequence_id)
                            .expect("stopped sequence missing")
                            .sever_following_link(reference.element_index);
                    }
                }
                Frame::Postponed(reference) => {
                    let postponed = self
                        .orders
                        .sequence_manager
                        .get_sequence(reference.sequence_id)
                        .expect("stopped sequence missing")
                        .live_postponed_ref(reference.element_index);
                    if let Some(postponed) = postponed {
                        frames.push(Frame::AfterPostponed(reference));
                        frames.push(Frame::Enter(postponed));
                    } else {
                        call_path.remove(&reference);
                    }
                }
                Frame::AfterPostponed(reference) => {
                    let current_postponed = self
                        .orders
                        .sequence_manager
                        .get_sequence(reference.sequence_id)
                        .expect("stopped sequence missing")
                        .live_postponed_ref(reference.element_index);
                    if current_postponed.is_some_and(|next| {
                        self.orders
                            .sequence_manager
                            .get_element(next.sequence_id, next.element_index)
                            .expect("postponed Stop target missing")
                            .state
                            == SequenceState::Interrupted
                    }) {
                        self.orders
                            .sequence_manager
                            .get_sequence_mut(reference.sequence_id)
                            .expect("stopped sequence missing")
                            .sever_postponed_link(reference.element_index);
                    }
                    call_path.remove(&reference);
                }
            }
        }
    }

    /// Stop all active and pending sequence elements owned by `owner` whose
    /// priority is weak enough to be pre-empted by `stop_priority`.
    ///
    /// Calls [`Sequence::stop_element`] on the actor's authoritative current
    /// element, then runs [`SequenceManager::stop_pending_elements`] for the
    /// not-yet-launched queue. Actor stopping does not scan every live
    /// element owned by the actor: it follows the selected sequence element, whose
    /// postponed chain remains reachable even while a terminating injury's
    /// condolence callback is running. Cross-sequence postponed work is the
    /// Rust representation of that same pointer and is stopped explicitly.
    pub(crate) fn stop_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) {
        let root = self.world.entities.current_element_for_actor(owner);
        self.stop_owner_from_root(
            sim,
            assets,
            active_scripts,
            owner,
            root,
            stop_priority,
            resolver,
        );
    }

    /// Stop an actor from an explicit root instead of the actor's
    /// currently selected element.
    ///
    /// The explicit root lets callers stop a retained graph branch while
    /// preserving the actor's current selection.
    pub(crate) fn stop_owner_from_root(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) {
        self.stop_owner_current_from_root(
            sim,
            assets,
            active_scripts,
            root,
            stop_priority,
            resolver,
        );
        self.stop_pending_elements(sim, assets, active_scripts, owner, stop_priority, resolver);
    }

    /// Stop only the actor-selected element and its postponed graph.
    ///
    /// Original-game actor stopping has an observable callback boundary:
    /// stopping the selected sequence element synchronously invokes
    /// removal notification, and only after that callback returns does the actor
    /// stop not-yet-launched sequence elements. Engine call sites which can
    /// pump that callback use this phase separately, then call
    /// [`SequenceManager::stop_pending_elements`] after the callback has completed.
    pub(crate) fn stop_owner_current_from_root(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) {
        let Some((sequence, index)) = root else {
            return;
        };
        if self
            .orders
            .sequence_manager
            .get_element(sequence, index)
            .is_some_and(|element| {
                element.command == Command::Wait && element.priority == SequencePriority::Wait
            })
        {
            return;
        }
        self.stop_live_sequence_element(
            sim,
            assets,
            active_scripts,
            SequenceElementRef::new(sequence, index),
            stop_priority,
            resolver,
            &mut HashSet::new(),
        );
    }

    /// Stop not-yet-launched elements for a specific actor up to a priority.
    pub(crate) fn stop_pending_elements(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) {
        // Work from the same roots as Original's
        // Stop not-yet-launched sequence elements: entries currently registered in
        // the manager's to-go list for this owner. A root can be too strong to
        // stop while still owning a postponed pointer; sequence stopping
        // follows that pointer unconditionally, so retain the cross-sequence
        // targets returned by Rust's split-storage representation.
        let roots = self
            .orders
            .sequence_manager
            .pending_elements_for_owner(owner);
        self.stop_pending_roots(
            sim,
            assets,
            active_scripts,
            owner,
            roots,
            stop_priority,
            resolver,
        );
        self.orders
            .sequence_manager
            .compact_terminal_elements_to_go();
    }

    pub(crate) fn stop_pending_roots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        owner: EntityId,
        roots: impl IntoIterator<Item = (SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) {
        for (sequence, index) in roots {
            if !self
                .orders
                .sequence_manager
                .get_element(sequence, index)
                .is_some_and(|element| element.owner == Some(owner))
            {
                continue;
            }
            self.stop_live_sequence_element(
                sim,
                assets,
                active_scripts,
                SequenceElementRef::new(sequence, index),
                stop_priority,
                resolver,
                &mut HashSet::new(),
            );
        }
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
    pub(crate) fn stop_movement_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        entity: EntityId,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            sim,
            assets,
            active_scripts,
            entity,
            None,
            owner_pos,
            stop_priority,
            resolver,
        )
    }

    /// Run the movement-specific stopping phase for exactly one
    /// selected element. This is the narrow counterpart to
    /// [`SequenceManager::stop_movement_for_owner`]: call sites modeling
    /// Stopping the selected element must not rewrite unrelated in-progress
    /// movements owned by the same actor.
    pub(crate) fn stop_movement_from_root(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        entity: EntityId,
        root: (SequenceId, usize),
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            sim,
            assets,
            active_scripts,
            entity,
            Some(root),
            owner_pos,
            stop_priority,
            resolver,
        )
    }

    pub(crate) fn stop_movement_for_owner_from_root(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
        entity: EntityId,
        root: Option<(SequenceId, usize)>,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&crate::engine::EngineInner, &SequenceElement) -> SequencePriority,
    ) -> bool {
        let mut changed = false;
        let mut to_interrupt: Vec<(SequenceId, usize)> = Vec::new();
        let refs: Vec<SequenceElementRef> = if let Some((sequence_id, element_index)) = root {
            vec![SequenceElementRef {
                sequence_id,
                element_index,
            }]
        } else {
            let Some(refs) = self.orders.sequence_manager.actor_live.get(&entity) else {
                return false;
            };
            refs.iter()
                .copied()
                .filter(|element_ref| {
                    self.orders
                        .sequence_manager
                        .get_element(element_ref.sequence_id, element_ref.element_index)
                        .expect("actor_live contains stale element ref")
                        .state
                        == SequenceState::InProgress
                })
                .collect()
        };
        for elem_ref in refs {
            let Some(seq) = self
                .orders
                .sequence_manager
                .sequences
                .get(&elem_ref.sequence_id)
            else {
                debug_assert!(false, "actor_live contains stale sequence ref");
                continue;
            };
            let seq_id = seq.id;
            let elem_idx = elem_ref.element_index;
            let Some(elem) = seq.elements.get(elem_idx) else {
                debug_assert!(false, "actor_live contains stale element ref");
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
            if self.resolve_live_sequence_stop_priority(
                SequenceElementRef::new(seq_id, elem_idx),
                resolver,
            ) < stop_priority
            {
                continue;
            }
            let elem = self
                .orders
                .sequence_manager
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
            first.reseed_id(crate::order::alloc_order_id(&mut self.orders.next_order_id));
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
            self.element_interrupted(
                sim,
                assets,
                active_scripts,
                seq_id,
                elem_idx,
                CascadeFlags::NEXT_LEVEL,
            );
            changed = true;
        }
        changed
    }
}
