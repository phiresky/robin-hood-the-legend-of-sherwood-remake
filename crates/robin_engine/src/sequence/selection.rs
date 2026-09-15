//! Sequence manager selection responsibilities.
use super::*;

impl SequenceManager {
    /// Resolve the current following chain after the owner's completion callback.
    pub(crate) fn live_cascade_target(
        &self,
        sequence_id: SequenceId,
        element_index: usize,
        flags: CascadeFlags,
    ) -> Option<SequenceElementRef> {
        let sequence = self
            .get_sequence(sequence_id)
            .expect("cascade owner sequence missing");
        if flags.contains(CascadeFlags::FOLLOWING) {
            return sequence.live_following_ref(element_index);
        }
        if !flags.contains(CascadeFlags::NEXT_LEVEL) {
            return None;
        }
        let command_level = self
            .get_element(sequence_id, element_index)
            .expect("cascade owner element missing")
            .command_level;
        let mut next = sequence.live_following_ref(element_index);
        let mut visited = HashSet::new();
        while let Some(target) = next {
            assert!(
                visited.insert(target),
                "cycle in following cascade at {target:?}"
            );
            let element = self
                .get_element(target.sequence_id, target.element_index)
                .expect("following cascade target missing");
            if element.command_level != command_level {
                return Some(target);
            }
            next = self
                .get_sequence(target.sequence_id)
                .expect("following sequence missing")
                .live_following_ref(target.element_index);
        }
        None
    }

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

        self.get_element_mut(sequence_id, element_index)
            .expect("element disappeared during owner reassignment")
            .owner = Some(new_owner);

        if Self::is_actor_live_state(state) {
            self.insert_actor_live_ref(new_owner, element_ref);
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

    // ─── Launch ─────────────────────────────────────────────────

    /// Assign stable identities and store a sequence before engine execution.
    pub fn insert_sequence(&mut self, mut sequence: Sequence) -> SequenceId {
        assert!(!sequence.is_empty(), "cannot launch an empty sequence");

        // Stamp a deterministic per-engine id over whatever the
        // `Sequence::new()` placeholder allocated. Counter advances
        // here so replay sees identical ids. Same treatment for each
        // element id — the global atomic in `SequenceElement::new`
        // was process-wide and broke rollback.
        let previous_id = sequence.id;
        sequence.id = SequenceId(self.next_sequence_id);
        self.next_sequence_id = self.next_sequence_id.wrapping_add(1);
        for element in sequence.elements.iter_mut() {
            for link in [&mut element.next, &mut element.postponed] {
                if let Some(target) = link
                    && target.sequence_id == previous_id
                {
                    target.sequence_id = sequence.id;
                }
            }
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

        self.sequences.insert(id, sequence);
        self.index_sequence_actor_refs(id);

        id
    }

    /// Store a single element as a sequence without registering its first level.
    pub fn insert_element(&mut self, mut element: SequenceElement) -> SequenceId {
        element.command_level = 1;
        let mut seq = Sequence::new();
        seq.append_element(element);
        self.insert_sequence(seq)
    }

    /// Register an owned command for instruction at the next FIFO boundary.
    pub(crate) fn register_owned_command(
        &mut self,
        actor: EntityId,
        command: Command,
    ) -> SequenceId {
        let elem = SequenceElement::new_generic(1, command, Some(actor));
        let id = self.insert_element(elem);
        self.sequences
            .get_mut(&id)
            .expect("inserted sequence")
            .next_elements_go();
        self.elements_to_go.push_back((id, 0));
        id
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

    /// Whether the actor's original-game-equivalent selected element currently
    /// names a movement element.
    pub fn actor_has_selected_movement<I: Into<EntityId>>(
        &self,
        entities: &crate::entities::Entities,
        actor: I,
    ) -> bool {
        entities
            .current_element_for_actor(actor)
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

    /// Find the first in-progress element owned by `actor` that
    /// satisfies `predicate`, filtering the actor's live elements.
    /// Lets callers check the actor's parallel in-progress elements
    /// without scanning every sequence in the manager.
    pub fn in_progress_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        let set = self.actor_live.get(&actor)?;
        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                panic!("actor_live contains stale element ref {elem_ref:?}");
            };
            if elem.state == SequenceState::InProgress && predicate(elem) {
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
        entities: &crate::entities::Entities,
        actor: I,
    ) -> Option<(SequenceId, usize, &Order)> {
        let (seq_id, elem_idx) = entities.current_element_for_actor(actor)?;
        let order = self.get_element(seq_id, elem_idx)?.current_order()?;
        Some((seq_id, elem_idx, order))
    }
}
