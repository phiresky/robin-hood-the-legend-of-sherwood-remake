//! Sequence manager registry responsibilities.
use super::*;

impl SequenceManager {
    /// Graph rewrites follow the same live edge as execution.
    pub(super) fn rewrite_following_ref(
        &self,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Option<(SequenceId, usize)> {
        let next = self
            .get_sequence(sequence_id)?
            .live_following_ref(element_index)?;
        Some((next.sequence_id, next.element_index))
    }

    pub(super) fn unsevered_following_ref(
        &self,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Option<(SequenceId, usize)> {
        let next = self
            .get_sequence(sequence_id)?
            .unsevered_following_ref(element_index)?;
        Some((next.sequence_id, next.element_index))
    }

    /// Ordered queue and selected-owner views used by the schema-13 parity
    /// snapshot. References remain in native IDs here; the engine facade maps
    /// them to manager insertion ordinals before exposing the snapshot.
    #[doc(hidden)]
    #[cfg(any(test, feature = "original-parity", feature = "test-helpers"))]
    pub(crate) fn parity_runtime_refs(
        &self,
        entities: &crate::entities::Entities,
    ) -> (
        Vec<(SequenceId, usize)>,
        Vec<(EntityId, SequenceElementRef)>,
    ) {
        if self.halt_pending {
            panic!("parity sequence capture reached a non-quiescent dispatch boundary");
        }
        let actor_current = entities
            .occupied()
            .filter_map(|(owner, entity)| {
                Some((owner, entity.actor_data()?.selected_sequence_element?))
            })
            .collect();
        (self.elements_to_go.iter().copied().collect(), actor_current)
    }
    pub(super) fn is_actor_live_state(state: SequenceState) -> bool {
        matches!(
            state,
            SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed
        )
    }

    pub(super) fn insert_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        self.get_element_at(elem_ref)
            .expect("cannot index missing live element");
        self.actor_live.entry(owner).or_default().insert(elem_ref);
    }

    pub(super) fn remove_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        if let Some(set) = self.actor_live.get_mut(&owner) {
            set.remove(&elem_ref);
            if set.is_empty() {
                self.actor_live.remove(&owner);
            }
        }
    }

    pub fn new() -> Self {
        Self {
            sequences: IndexMap::new().into(),
            actor_live: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            elements_to_go: VecDeque::new(),
            next_sequence_id: 1,
            next_element_id: 1,
            halt_pending: false,
        }
    }

    /// Atomically replace manager-owned state after every v48 identity and
    /// reference has been validated.
    pub(crate) fn restore_v48_state(&mut self, state: SequenceManagerV48State) {
        let mut restored = Self {
            sequences: state
                .sequences
                .into_iter()
                .map(|sequence| (sequence.id, sequence))
                .collect::<IndexMap<_, _>>()
                .into(),
            actor_live: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            elements_to_go: state.elements_to_go,
            next_sequence_id: state.next_sequence_id,
            next_element_id: state.next_element_id,
            halt_pending: false,
        };
        restored.rebuild_indices();
        *self = restored;
    }

    /// Rebuild the actor element indexes from `sequences`.  This is
    /// still useful after older save loads and defensive repair paths.
    /// `sequences` itself is serialized, and `BTreeMap` preserves ids
    /// across cleanup, so no index-shift rebuild is needed on the
    /// cleanup path.
    pub fn rebuild_indices(&mut self) {
        self.actor_live.clear();
        self.postpone_tail_cache.clear();
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                let Some(owner) = elem.owner else {
                    continue;
                };
                let elem_ref = SequenceElementRef::new(*seq_id, elem_idx);
                if Self::is_actor_live_state(elem.state) {
                    self.actor_live.entry(owner).or_default().insert(elem_ref);
                }
            }
        }
    }

    /// Toggle the halt-pending marker. While `true`, any
    /// [`CondolationCard`] delivered during a terminal transition will be
    /// tagged with `from_halt=true`. Callers bracket a
    /// `stop_owner(Preference)` invocation with
    /// `set_halt_pending(true) … set_halt_pending(false)` so handlers
    /// can detect the AI-initiated `Halt()` window.
    pub fn set_halt_pending(&mut self, v: bool) {
        self.halt_pending = v;
    }

    /// Number of active sequences.
    pub fn sequence_count(&self) -> usize {
        self.sequences.len()
    }
}
