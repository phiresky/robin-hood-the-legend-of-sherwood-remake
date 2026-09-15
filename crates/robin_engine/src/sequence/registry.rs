//! Sequence manager registry responsibilities.
use super::*;

impl SequenceManager {
    /// Graph rewrites historically follow stored edges even when the runtime
    /// severed-link mirror is set; owner filtering is performed at each visited
    /// node. Keep that distinct from live movement and completion queries.
    /// TODO(sequence-links): establish Original rewrite behavior for a retained
    /// runtime-authored severed edge before changing this query's semantics.
    pub(super) fn rewrite_following_ref(
        &self,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Option<(SequenceId, usize)> {
        let next = self
            .get_sequence(sequence_id)?
            .raw_following_ref(element_index)?;
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

    /// Resolve a local cascade's following reference, preserving loaded v48
    /// non-adjacent links and treating severed links as null. Cross-sequence
    /// targets are rejected because cascade effects currently use local indices.
    pub(crate) fn following_element_ref(
        &self,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Option<(SequenceId, usize)> {
        self.get_sequence(sequence_id)
            .and_then(|sequence| sequence.following_element_index(element_index))
            .map(|following_index| (sequence_id, following_index))
    }

    /// Ordered queue and selected-owner views used by the schema-13 parity
    /// snapshot. References remain in native IDs here; the engine facade maps
    /// them to manager insertion ordinals before exposing the snapshot.
    #[doc(hidden)]
    pub(crate) fn parity_runtime_refs(
        &self,
    ) -> (
        Vec<(SequenceId, usize)>,
        Vec<(EntityId, SequenceElementRef)>,
    ) {
        if !self.actor_instructing.is_empty()
            || self.actor_translating.is_some()
            || self.halt_pending
        {
            panic!("parity sequence capture reached a non-quiescent dispatch boundary");
        }
        let actor_current = self
            .actor_in_progress
            .iter()
            .filter_map(|(owner, refs)| refs.first().copied().map(|element| (*owner, element)))
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
        self.get_element(elem_ref.sequence_id, elem_ref.element_index)
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
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
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
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
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
        self.actor_in_progress.clear();
        self.actor_instructing.clear();
        self.actor_translating = None;
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                let Some(owner) = elem.owner else {
                    continue;
                };
                let elem_ref = SequenceElementRef::new(*seq_id, elem_idx);
                if Self::is_actor_live_state(elem.state) {
                    self.actor_live.entry(owner).or_default().insert(elem_ref);
                }
                if elem.state == SequenceState::InProgress {
                    self.actor_in_progress
                        .entry(owner)
                        .or_default()
                        .insert(elem_ref);
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
