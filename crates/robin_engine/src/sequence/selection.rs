//! Sequence manager selection responsibilities.
use super::*;

impl SequenceManager {
    // ─── Lookup ─────────────────────────────────────────────────

    /// Get a sequence by ID. O(log N).
    pub fn get_sequence(&self, id: SequenceId) -> Option<&Sequence> {
        self.sequences.get(&id)
    }

    /// Get a mutable sequence by ID. O(log N).
    pub fn get_sequence_mut(&mut self, id: SequenceId) -> Option<&mut Sequence> {
        self.sequences.get_mut(&id)
    }

    /// Reassign a live element at an owner instruction boundary.
    ///
    /// Original PC-on-shoulders movement changes the movement element's owner
    /// from rider to carrier only when the rider receives an instruction. Keep the
    /// derived actor indexes consistent with that pointer mutation.
    pub(crate) fn reassign_element_owner(
        &mut self,
        sequence_id: SequenceId,
        element_index: usize,
        new_owner: EntityId,
    ) {
        let element_ref = SequenceElementRef::new(sequence_id, element_index);
        let (old_owner, state) = self
            .get_element(sequence_id, element_index)
            .map(|element| (element.owner, element.state))
            .unwrap_or_else(|| {
                panic!("cannot reassign missing sequence element {sequence_id:?}/{element_index}")
            });
        let Some(old_owner) = old_owner else {
            panic!("cannot reassign ownerless sequence element {sequence_id:?}/{element_index}")
        };
        if old_owner == new_owner {
            return;
        }

        if Self::is_actor_live_state(state) {
            self.remove_actor_live_ref(old_owner, element_ref);
        }
        if state == SequenceState::InProgress
            && let Some(set) = self.actor_in_progress.get_mut(&old_owner)
        {
            set.remove(&element_ref);
            if set.is_empty() {
                self.actor_in_progress.remove(&old_owner);
            }
        }

        self.get_element_mut(sequence_id, element_index)
            .expect("element disappeared during owner reassignment")
            .owner = Some(new_owner);

        if Self::is_actor_live_state(state) {
            self.insert_actor_live_ref(new_owner, element_ref);
        }
        if state == SequenceState::InProgress {
            self.actor_in_progress
                .entry(new_owner)
                .or_default()
                .insert(element_ref);
        }
    }

    pub(super) fn index_sequence_actor_refs(&mut self, seq_id: SequenceId) {
        let refs: Vec<(EntityId, SequenceElementRef, SequenceState)> = {
            let Some(seq) = self.sequences.get(&seq_id) else {
                return;
            };
            seq.elements
                .iter()
                .enumerate()
                .filter_map(|(elem_idx, elem)| {
                    elem.owner
                        .map(|owner| (owner, SequenceElementRef::new(seq_id, elem_idx), elem.state))
                })
                .collect()
        };

        for (owner, elem_ref, state) in refs {
            if Self::is_actor_live_state(state) {
                self.insert_actor_live_ref(owner, elem_ref);
            }
            if state == SequenceState::InProgress {
                self.actor_in_progress
                    .entry(owner)
                    .or_default()
                    .insert(elem_ref);
            }
        }
    }

    /// Read-only iterator over every sequence currently owned by the
    /// manager. Used by engine-layer helpers that need to locate an
    /// actor's currently-executing element across all sequences — we
    /// don't keep a back-pointer on each actor.
    pub fn sequences_iter(&self) -> impl Iterator<Item = &Sequence> + '_ {
        self.sequences.values()
    }

    /// Install an authoritative replay route on the one pending point Seek
    /// created by DropAle. Original computes this route only when the
    /// postponed Seek is instructed, potentially many frames after the input
    /// command was recorded.
    pub(crate) fn inject_recorded_drop_ale_route(
        &mut self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
        goal_sector: crate::position_interface::SectorHandle,
        goal_layer: u16,
        recorded_gate_path: crate::gate::RecordedGatePath,
    ) -> Result<(), String> {
        let elements_to_go = &self.elements_to_go;
        let candidates =
            self.sequences
                .iter()
                .flat_map(|(sequence_id, sequence)| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            let SequenceElementData::Movement {
                                destination: element_destination,
                                element: target,
                                flags,
                                post_seek_sequence,
                                ..
                            } = &element.data
                            else {
                                return None;
                            };
                            let is_drop_ale =
                                post_seek_sequence.as_ref().is_some_and(|post_seek| {
                                    post_seek.elements.first().is_some_and(|post_element| {
                                        post_element.command == Command::DropAle
                                    })
                                });
                            if element.owner != Some(actor)
                                || element.command != Command::Seek
                                || !(element.state == SequenceState::Postponed
                                    || elements_to_go.contains(&(*sequence_id, element_index)))
                                || element.point_seek_route_provenance
                                    != PointSeekRouteProvenance::OriginalReplay
                                || target.is_some()
                                || !flags.contains(MoveFlags::SEEK)
                                || !is_drop_ale
                                || element_destination.x.to_bits() != destination.x.to_bits()
                                || element_destination.y.to_bits() != destination.y.to_bits()
                            {
                                return None;
                            }
                            Some((*sequence_id, element_index))
                        },
                    )
                })
                .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(format!(
                "matched {} pending point Seeks at destination ({}, {})",
                candidates.len(),
                destination.x,
                destination.y,
            ));
        }
        let (sequence_id, element_index) = candidates[0];
        let element = self
            .get_element_mut(sequence_id, element_index)
            .expect("recorded DropAle route candidate disappeared before admission");
        if element.point_seek_route_provenance != PointSeekRouteProvenance::OriginalReplay {
            return Err("matching DropAle seek is not owned by Original replay".to_owned());
        }
        if element.recorded_gate_path.is_some() {
            return Err("matching DropAle seek already has a recorded gate route".to_owned());
        }
        let SequenceElementData::Movement { layer, sector, .. } = &mut element.data else {
            unreachable!("recorded DropAle route candidate stopped being movement")
        };
        *sector = Some(goal_sector);
        *layer = goal_layer;
        element.recorded_gate_path = Some(recorded_gate_path);
        Ok(())
    }

    pub(crate) fn has_pending_drop_ale_route_candidate(
        &self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> bool {
        let candidates = self
            .sequences
            .iter()
            .flat_map(|(sequence_id, sequence)| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .map(move |(element_index, element)| (*sequence_id, element_index, element))
            })
            .filter_map(|(sequence_id, element_index, element)| {
                let SequenceElementData::Movement {
                    destination: element_destination,
                    element: target,
                    flags,
                    post_seek_sequence,
                    ..
                } = &element.data
                else {
                    return None;
                };
                let is_drop_ale = post_seek_sequence.as_ref().is_some_and(|post_seek| {
                    post_seek
                        .elements
                        .first()
                        .is_some_and(|post_element| post_element.command == Command::DropAle)
                });
                if element.owner != Some(actor)
                    || element.command != Command::Seek
                    || !(element.state == SequenceState::Postponed
                        || self.elements_to_go.contains(&(sequence_id, element_index)))
                    || element.point_seek_route_provenance
                        != PointSeekRouteProvenance::OriginalReplay
                    || target.is_some()
                    || !flags.contains(MoveFlags::SEEK)
                    || !is_drop_ale
                    || element_destination.x.to_bits() != destination.x.to_bits()
                    || element_destination.y.to_bits() != destination.y.to_bits()
                {
                    return None;
                }
                Some(element.recorded_gate_path.is_some())
            })
            .collect::<Vec<_>>();
        assert!(
            candidates.len() <= 1,
            "recorded DropAle route matched {} pending point Seeks for {actor:?}",
            candidates.len()
        );
        if let Some(already_recorded) = candidates.first() {
            assert!(
                !*already_recorded,
                "pending DropAle point Seek already has a recorded gate route"
            );
            true
        } else {
            false
        }
    }

    pub(crate) fn is_registered_to_go(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.elements_to_go.contains(&(seq_id, elem_idx))
            && self
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| element.state != SequenceState::Interrupted)
    }

    /// Snapshot the deferred manager FIFO without changing registration.
    /// Synchronous engine boundaries use this to identify only the elements
    /// authored by a nested statement while leaving older and foreign-owner
    /// work in place.
    pub(crate) fn deferred_elements_to_go(&self) -> Vec<(SequenceId, usize)> {
        self.elements_to_go.iter().copied().collect()
    }

    #[cfg(test)]
    pub(crate) fn v48_elements_to_go(&self) -> Vec<(SequenceId, usize)> {
        self.deferred_elements_to_go()
    }

    /// Get a reference to a specific element within a sequence.
    pub fn get_element(&self, seq_id: SequenceId, elem_idx: usize) -> Option<&SequenceElement> {
        self.get_sequence(seq_id)?.get(elem_idx)
    }

    /// Get a mutable reference to a specific element.
    pub fn get_element_mut(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Option<&mut SequenceElement> {
        self.get_sequence_mut(seq_id)?.get_mut(elem_idx)
    }

    /// Drop queue-time movement-goal snapshots held by live work for an
    /// actor whose outgoing movement genuinely exhausted. Those snapshots
    /// only bridge an interrupted replacement handoff; they must not revive
    /// a goal cleared by ordinary movement completion.
    pub(crate) fn clear_retained_movement_goals_for_actor(&mut self, actor: EntityId) {
        let live: Vec<_> = self
            .actor_live
            .get(&actor)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        for element_ref in live {
            let element = self
                .get_element_mut(element_ref.sequence_id, element_ref.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "actor_live contains stale element ref {:?}/{}",
                        element_ref.sequence_id, element_ref.element_index
                    )
                });
            // Replacement movements use the typed cache, while deferred
            // Facing commands store the same snapshot as a Generic property until
            // the manager instructs its Turn. Both are Rust mirrors of
            // the one original-game map-position goal owned and cleared by
            // the actor's completion callback.
            element.retained_movement_goal = None;
            element.remove_property(Field::RetainedMovementGoal);
        }
    }

    // ─── Launch ─────────────────────────────────────────────────

    /// Launch a fully-built sequence. Returns its ID.
    pub fn launch_sequence(&mut self, mut sequence: Sequence) -> SequenceId {
        assert!(!sequence.is_empty(), "cannot launch an empty sequence");

        // Stamp a deterministic per-engine id over whatever the
        // `Sequence::new()` placeholder allocated. Counter advances
        // here so replay sees identical ids. Same treatment for each
        // element id — the global atomic in `SequenceElement::new`
        // was process-wide and broke rollback.
        sequence.id = SequenceId(self.next_sequence_id);
        self.next_sequence_id = self.next_sequence_id.wrapping_add(1);
        for element in sequence.elements.iter_mut() {
            element.id = self.next_element_id;
            self.next_element_id = self.next_element_id.wrapping_add(1);
        }
        let id = sequence.id;
        tracing::trace!(
            sequence_id = id.0,
            elements = ?sequence
                .elements
                .iter()
                .map(|element| (
                    element.owner,
                    element.command,
                    element.command_level,
                    element.state,
                    element.priority,
                    &element.data,
                ))
                .collect::<Vec<_>>(),
            "launching sequence"
        );
        sequence.launch();

        // Start the first batch of elements
        let to_go = sequence.next_elements_go();

        self.sequences.insert(id, sequence);
        self.index_sequence_actor_refs(id);

        // Register elements for dispatch, one loop iteration at a time.
        self.register_level_elements_to_go(id, to_go);

        id
    }

    /// Launch a single sequence element by wrapping it in a new sequence.
    pub fn launch_element(&mut self, mut element: SequenceElement) -> SequenceId {
        element.command_level = 1;
        let mut seq = Sequence::new();
        seq.append_element(element);
        self.launch_sequence(seq)
    }

    /// Interrupt one freshly launched actor Wait before its synchronous
    /// launch action reaches instruction handling.
    ///
    /// This is deliberately identity-based rather than an owner/command scan:
    /// EnterBeggar's DONE callback creates one Wait, postpones it behind the
    /// still-selected noninterruptible transition, then selected-PC
    /// `SelectAction(Beggar)` immediately stops that exact postponed element.
    /// Rust replays the callback after retiring the transition, so its split
    /// representation must remove the queued instruction before it can select
    /// the Wait. Preserve the launch (and therefore sequence/element ID
    /// consumption) while touching no older queued work for the same owner.
    pub(crate) fn interrupt_just_registered_wait_before_instruct(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
    ) {
        let element = self
            .get_element(sequence_id, 0)
            .unwrap_or_else(|| panic!("fresh Wait {sequence_id:?}/0 disappeared before Stop"));
        assert_eq!(
            element.owner,
            Some(owner),
            "fresh Wait {sequence_id:?}/0 changed owner before Stop"
        );
        assert_eq!(
            element.command,
            Command::Wait,
            "selected beggar callback may discard only its fresh Wait"
        );
        assert_eq!(
            element.priority,
            SequencePriority::Wait,
            "selected beggar callback Wait lost RHPRIORITY_WAIT"
        );
        assert_eq!(
            element.state,
            SequenceState::Todo,
            "selected beggar callback Wait must be stopped before Instruct"
        );
        assert!(
            element.orders.is_empty(),
            "selected beggar callback Wait translated before its Stop"
        );

        let target = (sequence_id, 0);
        let queued_actions = self
            .pending_synchronous_actions
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    PendingSyncEntry::Action(SequenceAction::InstructOwner {
                        owner: queued_owner,
                        sequence_id: queued_sequence,
                        element_index: 0,
                    }) if *queued_owner == owner && *queued_sequence == sequence_id
                )
            })
            .count();
        assert_eq!(
            queued_actions, 1,
            "fresh Wait {sequence_id:?}/0 must have exactly one queued Go action"
        );
        self.pending_synchronous_actions.retain(|entry| {
            !matches!(
                entry,
                PendingSyncEntry::Action(SequenceAction::InstructOwner {
                    owner: queued_owner,
                    sequence_id: queued_sequence,
                    element_index: 0,
                }) if *queued_owner == owner && *queued_sequence == sequence_id
            )
        });
        assert!(
            !self.elements_to_go.contains(&target),
            "fresh RHPRIORITY_WAIT element unexpectedly entered the deferred manager FIFO"
        );
        assert!(
            self.terminate_sequence(sequence_id),
            "fresh Wait {sequence_id:?} disappeared before interruption"
        );
    }

    /// Launch a one-shot generic sequence carrying a single pre-built
    /// `Order` for `actor`, and immediately mark its element as
    /// `InProgress` so consumers (animation driver, AI peek-current)
    /// see it this frame rather than waiting for the next
    /// `hourglass` dispatch. Used by swordfight entry /
    /// `QuitSwordfight` / `process_pending_ai_orders` to build a
    /// generic element, push the order onto its `orders` queue, then
    /// launch with priority resolution firing synchronously.  Keeping
    /// every in-flight `Order` attached to an `InProgress` element
    /// means cancellation (via `set_element_state`) naturally discards
    /// the orders along with the element.
    ///
    /// Suffixed `_unchecked` because this path bypasses the instruction
    /// equivalent (posture/action-state stamp + priority arbitration
    /// against the actor's current element).  Every caller except
    /// `EngineInner::launch_single_order_sequence_stamped` should go
    /// through that wrapper; the `_unchecked` form is kept only for
    /// the stamped wrapper's internals.  A grep for this name should
    /// turn up exactly one caller.
    pub(crate) fn launch_single_order_sequence_unchecked(
        &mut self,
        actor: EntityId,
        command: Command,
    ) -> SequenceId {
        // Launch the empty element.  The caller (always
        // `EngineInner::launch_single_order_sequence_stamped`) is
        // responsible for running instruction handling (posture
        // stamp + `generate_transition` + arbitration) and THEN
        // appending the pre-baked single order.  Ordering matters:
        // `generate_transition` (exit + posture + enter) populates the
        // order queue BEFORE `Translate` pushes the command's own
        // order, so those transitions play first.
        let elem = SequenceElement::new_generic(1, command, Some(actor));
        self.launch_element(elem)
    }

    /// Push an `Order` onto the given element.  Panics if the handle is
    /// stale — callers must hold a live `(seq_id, elem_idx)` for an
    /// element they just launched or are currently dispatching, so a
    /// `None` here means a bug upstream, not a recoverable race.
    pub fn push_order_on(&mut self, seq_id: SequenceId, elem_idx: usize, order: Order) {
        match self.get_element_mut(seq_id, elem_idx) {
            Some(elem) => elem.push_order(order),
            None => panic!(
                "push_order_on: no element at ({:?}, {}) — handle is stale",
                seq_id, elem_idx
            ),
        }
    }

    /// Drop every queued `Order` on the given element, keeping the element
    /// itself live.  Panics on a stale handle for the same reason
    /// [`push_order_on`](Self::push_order_on) does.
    pub fn clear_orders_on(&mut self, seq_id: SequenceId, elem_idx: usize) {
        match self.get_element_mut(seq_id, elem_idx) {
            Some(elem) => elem.orders.clear(),
            None => panic!(
                "clear_orders_on: no element at ({:?}, {}) — handle is stale",
                seq_id, elem_idx
            ),
        }
    }

    /// Find the actor's in-progress sequence element.  O(log k) via
    /// [`actor_in_progress`](Self::actor_in_progress), where k is the
    /// number of simultaneously-`InProgress` elements owned by this
    /// actor (typically 1; briefly 2 during cascades).  When an idle
    /// `Wait` overlaps a real command, the real command is the actor's
    /// current element; otherwise old idle waits could starve combat
    /// elements that should be the actor's current sequence element.
    pub fn current_element_for_actor<I: Into<EntityId>>(
        &self,
        actor: I,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        if let Some((elem_ref, false)) = self
            .actor_instructing
            .get(&actor)
            .and_then(|stack| stack.last())
        {
            return Some((elem_ref.sequence_id, elem_ref.element_index));
        }
        if let Some((owner, elem_ref)) = self.actor_translating
            && owner == actor
        {
            return Some((elem_ref.sequence_id, elem_ref.element_index));
        }
        let set = self.actor_in_progress.get(&actor)?;
        let mut refs = set.iter();
        let first = *refs.next()?;
        if refs.next().is_none() {
            return Some((first.sequence_id, first.element_index));
        }

        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if elem.command != Command::Wait {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        Some((first.sequence_id, first.element_index))
    }

    /// Whether the actor's original-game-equivalent selected element currently
    /// names a movement element.
    pub fn actor_has_selected_movement<I: Into<EntityId>>(&self, actor: I) -> bool {
        self.current_element_for_actor(actor)
            .and_then(|(sequence_id, element_index)| self.get_element(sequence_id, element_index))
            .is_some_and(|element| element.data.is_movement())
    }

    /// Furthest currently-live movement destination for an actor.  Shift-held
    /// planning uses this as the hypothetical origin when no queued move is
    /// already ahead of it.
    pub fn actor_planned_movement_destination(
        &self,
        actor: impl Into<EntityId>,
    ) -> Option<crate::coordinates::MapPoint> {
        let actor = actor.into();
        let live = self.actor_live.get(&actor)?;
        live.iter().rev().find_map(|element_ref| {
            let element = self.get_element(element_ref.sequence_id, element_ref.element_index)?;
            match &element.data {
                SequenceElementData::Movement { destination, .. } => Some(*destination),
                _ => None,
            }
        })
    }

    /// Select the accepted element for the duration of its command
    /// translation, or release it again.
    ///
    /// Releasing before a terminal state change reproduces the original game's
    /// post-translation clearing of the selected element for an accepted element whose
    /// translation produced no orders: that card must not claim the actor's
    /// movement goal, while a card raised from inside the translation body
    /// must.
    pub(crate) fn set_translating_element(
        &mut self,
        selection: Option<(EntityId, SequenceElementRef)>,
    ) {
        self.actor_translating = selection;
    }

    /// Read-only exposure for the opt-in goal/condolence ownership trace.
    pub(crate) fn goal_owner_debug_translating(&self) -> Option<(EntityId, SequenceElementRef)> {
        self.actor_translating
    }

    /// Release the translation selection when its own element is the one that
    /// just detached the actor's selected sequence element.
    ///
    /// The actor completion callback clears the selected element
    /// whenever the terminal element is the selected one
    /// when the terminal element is selected. A command body can reach that state
    /// from inside its own translation — path request insertion calls
    /// `Stop()` on the actor whose Move is being translated
    /// during path request insertion — and everything the same translation does
    /// afterwards, including the `Wait()` it launches next, must observe the
    /// cleared pointer. Rust holds the translation identity until after the
    /// deferred condolence dispatch, so drop it here instead.
    pub(crate) fn clear_translating_element_if_selected(
        &mut self,
        actor: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        if self.actor_translating == Some((actor, SequenceElementRef::new(seq_id, elem_idx))) {
            self.actor_translating = None;
        }
    }

    /// Select an incoming element while the outgoing element's synchronous
    /// interruption callback runs.
    ///
    /// The original game stores this selection in one raw element reference
    /// for the active element. A recursively accepted instruction
    /// overwrites that pointer permanently; returning from the recursive call
    /// does not restore its caller's selection. Keep the stack only to pair
    /// Rust callback scopes, and mark the parent superseded whenever a nested
    /// selection is installed.
    pub(crate) fn begin_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) {
        let stack = self.actor_instructing.entry(owner).or_default();
        if let Some((_, superseded)) = stack.last_mut() {
            *superseded = true;
        }
        stack.push((SequenceElementRef::new(sequence_id, element_index), false));
    }

    /// Close a matching [`Self::begin_instruct_callback`] boundary, returning
    /// whether recursive work left this element selected. This is Original's
    /// post-priority callback selection check.
    pub(crate) fn end_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> bool {
        let expected = SequenceElementRef::new(sequence_id, element_index);
        let stack = self
            .actor_instructing
            .get_mut(&owner)
            .unwrap_or_else(|| panic!("missing Instruct callback selection for {owner:?}"));
        let (selected, superseded) = stack
            .pop()
            .expect("Instruct callback selection stack is empty");
        assert_eq!(
            selected, expected,
            "Instruct callback selection closed out of order"
        );
        if stack.is_empty() {
            self.actor_instructing.remove(&owner);
        }
        !superseded
    }

    /// Find the first in-progress element owned by `actor` that
    /// satisfies `predicate`, using the same actor index as
    /// [`current_element_for_actor`](Self::current_element_for_actor).
    /// Lets callers check the actor's parallel in-progress elements
    /// without scanning every sequence in the manager.
    pub fn in_progress_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        let set = self.actor_in_progress.get(&actor)?;
        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if predicate(elem) {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        None
    }

    /// Returns true when `actor` owns a not-yet-terminal sequence element
    /// whose command matches `predicate`.
    pub fn has_live_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(Command) -> bool,
    ) -> bool {
        self.live_element_for_actor_matching(actor, |elem| predicate(elem.command))
            .is_some()
    }

    pub fn live_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        let set = self.actor_live.get(&actor)?;
        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_live contains stale element ref");
                continue;
            };
            if predicate(elem) {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        None
    }

    /// Returns true when `actor` owns a Todo or InProgress element
    /// whose command matches `predicate`.  Unlike
    /// [`Self::has_live_element_for_actor_matching`], this deliberately
    /// ignores `Postponed` elements: swordfight evaluation checks the
    /// actor's current animation, so a queued/postponed wait-priority
    /// smalltalk element must not suppress fresh smalltalk forever.
    pub fn has_unpostponed_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(Command) -> bool,
    ) -> bool {
        let actor = actor.into();
        let Some(set) = self.actor_live.get(&actor) else {
            return false;
        };
        set.iter().any(|elem_ref| {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_live contains stale element ref");
                return false;
            };
            matches!(elem.state, SequenceState::Todo | SequenceState::InProgress)
                && predicate(elem.command)
        })
    }

    /// Returns true when `actor` owns a Todo or InProgress element
    /// whose full element data matches `predicate`.
    pub fn has_unpostponed_element_for_actor_matching_element(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> bool {
        self.live_element_for_actor_matching(actor, |elem| {
            matches!(elem.state, SequenceState::Todo | SequenceState::InProgress) && predicate(elem)
        })
        .is_some()
    }

    /// Peek the actor's current in-progress order — the `Order` at the
    /// front of the owning `SequenceElement`'s `orders` queue.
    pub fn current_order_for_actor<I: Into<EntityId>>(
        &self,
        actor: I,
    ) -> Option<(SequenceId, usize, &Order)> {
        let (seq_id, elem_idx) = self.current_element_for_actor(actor)?;
        let order = self.get_element(seq_id, elem_idx)?.current_order()?;
        Some((seq_id, elem_idx, order))
    }
}
