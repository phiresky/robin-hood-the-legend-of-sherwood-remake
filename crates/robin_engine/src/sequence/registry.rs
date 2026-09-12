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
        if !self.pending_synchronous_actions.is_empty()
            || !self.pending_condolations.is_empty()
            || !self.actor_instructing.is_empty()
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
        let (priority, cross_only) = self
            .get_sequence(elem_ref.sequence_id)
            .unwrap_or_else(|| {
                panic!(
                    "cannot index missing live sequence {:?}",
                    elem_ref.sequence_id
                )
            })
            .elements
            .get(elem_ref.element_index)
            .map(|element| {
                (
                    element.priority,
                    self.get_sequence(elem_ref.sequence_id)
                        .and_then(|sequence| {
                            sequence.following_element_index(elem_ref.element_index)
                        })
                        .is_none()
                        && element.postponed_element_index.is_none(),
                )
            })
            .unwrap_or_else(|| {
                panic!(
                    "cannot index missing live element {:?}/{}",
                    elem_ref.sequence_id, elem_ref.element_index
                )
            });
        let already_live = self.actor_live.contains_key(&owner);
        self.actor_live.entry(owner).or_default().insert(elem_ref);
        if !already_live {
            self.actor_stop_summaries.insert(
                owner,
                ActorStopSummary {
                    weakest_priority: priority,
                    cross_only,
                },
            );
        } else if let Some(summary) = self.actor_stop_summaries.get_mut(&owner) {
            summary.weakest_priority = summary.weakest_priority.max(priority);
            summary.cross_only &= cross_only;
        }
    }

    pub(super) fn remove_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        let removed_priority = self
            .get_element(elem_ref.sequence_id, elem_ref.element_index)
            .map(|element| element.priority);
        if let Some(set) = self.actor_live.get_mut(&owner) {
            set.remove(&elem_ref);
            if set.is_empty() {
                self.actor_live.remove(&owner);
            }
        }
        // Recomputing here can turn a linear stop cascade into quadratic
        // work. Invalidate only when the removed element may have supplied
        // the ceiling; the next Stop query rebuilds it once if needed.
        if removed_priority.is_some_and(|priority| {
            self.actor_stop_summaries
                .get(&owner)
                .is_some_and(|summary| summary.weakest_priority == priority)
        }) {
            self.actor_stop_summaries.remove(&owner);
        }
    }

    pub(super) fn actor_stop_summary(&mut self, owner: EntityId) -> Option<ActorStopSummary> {
        if let Some(summary) = self.actor_stop_summaries.get(&owner) {
            return Some(*summary);
        }
        let summary = self.actor_live.get(&owner).and_then(|refs| {
            refs.iter().try_fold(
                ActorStopSummary {
                    weakest_priority: SequencePriority::NonInterruptable,
                    cross_only: true,
                },
                |mut summary, element_ref| {
                    let sequence =
                        self.get_sequence(element_ref.sequence_id)
                            .unwrap_or_else(|| {
                                panic!(
                                    "actor_live contains stale sequence ref {:?}",
                                    element_ref.sequence_id
                                )
                            });
                    let element = sequence
                        .elements
                        .get(element_ref.element_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "actor_live contains stale element ref {:?}/{}",
                                element_ref.sequence_id, element_ref.element_index
                            )
                        });
                    summary.weakest_priority = summary.weakest_priority.max(element.priority);
                    summary.cross_only &= sequence
                        .following_element_index(element_ref.element_index)
                        .is_none()
                        && element.postponed_element_index.is_none();
                    Some(summary)
                },
            )
        });
        if let Some(summary) = summary {
            self.actor_stop_summaries.insert(owner, summary);
        }
        summary
    }

    pub fn new() -> Self {
        Self {
            sequences: IndexMap::new().into(),
            actor_live: BTreeMap::new(),
            actor_stop_summaries: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            stop_noop_cache: BTreeMap::new(),
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
            elements_to_go: VecDeque::new(),
            pending_synchronous_actions: VecDeque::new(),
            pending_condolations: Vec::new(),
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
            actor_stop_summaries: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            stop_noop_cache: BTreeMap::new(),
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
            elements_to_go: state.elements_to_go,
            pending_synchronous_actions: VecDeque::new(),
            pending_condolations: Vec::new(),
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
        self.actor_stop_summaries.clear();
        self.postpone_tail_cache.clear();
        self.stop_noop_cache.clear();
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
                    let cross_only = seq.following_element_index(elem_idx).is_none()
                        && elem.postponed_element_index.is_none();
                    self.actor_stop_summaries
                        .entry(owner)
                        .and_modify(|summary| {
                            summary.weakest_priority = summary.weakest_priority.max(elem.priority);
                            summary.cross_only &= cross_only;
                        })
                        .or_insert(ActorStopSummary {
                            weakest_priority: elem.priority,
                            cross_only,
                        });
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
    /// [`PendingCondolation`] queued via `process_effects` will be
    /// tagged with `from_halt=true`. Callers bracket a
    /// `stop_owner(Preference)` invocation with
    /// `set_halt_pending(true) … set_halt_pending(false)` so handlers
    /// can detect the AI-initiated `Halt()` window.
    pub fn set_halt_pending(&mut self, v: bool) {
        self.halt_pending = v;
    }

    /// Drain all pending removal notifications accumulated
    /// since the last call.  EngineInner calls this after each `hourglass`
    /// and dispatches to per-entity cleanup handlers.
    pub fn drain_pending_condolations(&mut self) -> Vec<PendingCondolationDispatch> {
        std::mem::take(&mut self.pending_condolations)
    }

    /// Whether Original's synchronous NPC condolence callback already owns
    /// this owner's `EVENT_COULDNT_REACHPOINT` delivery.
    ///
    /// Marking a sequence element impossible
    /// sends its completion callback before it returns,
    /// and the NPC override dispatches this event for a final Move/MoveOk
    /// action. This implementation suspends that callback
    /// in `pending_condolations`, so an engine decision-tick completion must not
    /// overtake it with a second completion event.
    pub fn has_pending_couldnt_reachpoint_condolation(&self, owner: EntityId) -> bool {
        self.pending_condolations.iter().any(|pending| {
            let card = pending.card;
            card.owner == owner
                && !card.from_halt
                && !card.postponed_successor_pending
                && card.terminal_state == SequenceState::Impossible
                && matches!(
                    card.command,
                    Command::PassDoor | Command::Move | Command::MoveOk | Command::SitDown
                )
                && self.is_last_real_action(card.seq_id, usize::from(card.elem_idx))
        })
    }

    /// Restore a backlog detached around an owner-local synchronous boundary.
    /// The detached cards predate anything still queued, so they retain their
    /// original position at the front of the global FIFO.
    pub fn restore_pending_condolations(&mut self, mut pending: Vec<PendingCondolationDispatch>) {
        pending.append(&mut self.pending_condolations);
        self.pending_condolations = pending;
    }

    /// Drain only the pending condolations whose `owner` matches `owner`.
    /// Used by the per-NPC synchronous drain pass that runs right after
    /// each [`EngineInner::dispatch_filtered_stimulus`] — so a sequence
    /// that a handler's side effects just preempted fires its
    /// `Think(EVENT_DONE)` within the same call stack as the outer
    /// `Think` (re-entrant Think timing).  Condolations belonging to
    /// other entities remain queued for the end-of-tick global drain.
    pub fn drain_pending_condolations_for_owner(
        &mut self,
        owner: EntityId,
    ) -> Vec<PendingCondolationDispatch> {
        let mut matching = Vec::new();
        self.pending_condolations.retain(|c| {
            if c.card.owner == owner {
                matching.push(c.clone());
                false
            } else {
                true
            }
        });
        matching
    }

    /// Resume the state transition after
    /// removal notification: cascade, readiness, and postponed-element
    /// activation.  The engine calls this only after the card's recursive
    /// `Think` and its same-frame side effects have reached a fixed point.
    pub fn finish_pending_condolation(&mut self, pending: PendingCondolationDispatch) {
        let was_halt_pending = self.halt_pending;
        self.halt_pending |= pending.card.from_halt;
        self.process_effects_after_condolation(pending.card.seq_id, pending.effects_after_card);
        self.halt_pending = was_halt_pending;
    }

    /// Suspend a cross-postponed replacement at the same callback boundary as
    /// an interrupted predecessor. This is the continuation of the outer
    /// actor-instruction priority arbitration, not a successor released by the
    /// interrupted element itself.
    pub fn install_cross_postponed_after_condolation(
        &mut self,
        interrupted: (SequenceId, usize),
        blocker: (SequenceId, usize),
        waiter: (SequenceId, usize),
    ) {
        let pending = self
            .pending_condolations
            .iter_mut()
            .rev()
            .find(|pending| {
                pending.card.seq_id == interrupted.0
                    && usize::from(pending.card.elem_idx) == interrupted.1
            })
            .unwrap_or_else(|| {
                panic!(
                    "interrupted postponed element {:?}/{} produced no condolence continuation",
                    interrupted.0, interrupted.1
                )
            });
        assert!(
            pending
                .effects_after_card
                .install_cross_postponed_after_card
                .is_none(),
            "interrupted postponed element {:?}/{} already has a deferred cross install",
            interrupted.0,
            interrupted.1
        );
        pending
            .effects_after_card
            .install_cross_postponed_after_card = Some((blocker.0, blocker.1, waiter.0, waiter.1));
    }

    /// Number of active sequences.
    pub fn sequence_count(&self) -> usize {
        self.sequences.len()
    }
}
