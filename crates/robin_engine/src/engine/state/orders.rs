use crate::element::EntityId;
use crate::sequence::SequenceManager;

use super::super::{TimerEntry, movement};

/// Sequence graphs, timed work, and ordered path requests.
#[derive(
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct OrderRuntime {
    pub(crate) next_order_id: u32,
    pub(crate) pending_path_requests: movement::PendingPathRequestQueue,
    pub(crate) failed_path_requests: Vec<movement::FailedPathRequest>,
    pub(crate) timer_elements: Vec<TimerEntry>,
    pub(crate) sequence_manager: SequenceManager,
}

impl OrderRuntime {
    pub(crate) fn persisted_clone(&self) -> Self {
        let value = self;
        let OrderRuntime {
            next_order_id: _,
            pending_path_requests: _,
            failed_path_requests: _,
            timer_elements: _,
            sequence_manager: _,
        } = value;
        Self {
            next_order_id: value.next_order_id,
            pending_path_requests: value.pending_path_requests.clone(),
            failed_path_requests: value.failed_path_requests.clone(),
            timer_elements: value.timer_elements.clone(),
            sequence_manager: value.sequence_manager.persisted_clone(),
        }
    }
}

impl OrderRuntime {
    /// Cancel movement involving a retired actor. The path queue owns its
    /// logical-head timing; do not rebuild it or bypass its completion slot.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.failed_path_requests
            .retain(|request| !request.request.references_entity(id));
        self.pending_path_requests.remove_entity(id);
    }

    pub(crate) fn new() -> Self {
        Self {
            next_order_id: 1,
            pending_path_requests: Default::default(),
            failed_path_requests: Vec::new(),
            timer_elements: Vec::new(),
            sequence_manager: SequenceManager::new(),
        }
    }

    pub(crate) fn allocate_order_id(&mut self) -> std::num::NonZeroU32 {
        crate::order::alloc_order_id(&mut self.next_order_id)
    }

    /// Borrow one sequence element together with the order-id counter, so
    /// order rewrites on that element can stamp fresh ids.
    pub(crate) fn element_with_order_ids_mut(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<(&mut crate::sequence::SequenceElement, &mut u32)> {
        let Self {
            next_order_id,
            sequence_manager,
            ..
        } = self;
        sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .map(|element| (element, next_order_id))
    }

    /// Split the exact scheduler-owned leaves used by the path barrier.
    ///
    /// The sequence manager is read-only here. Path completion consequences
    /// (sequence mutation, hero speech, and condolation dispatch) remain owned
    /// by the root tick coordinator at their original-game evaluation points.
    pub(in crate::engine) fn path_schedule_parts(
        &mut self,
    ) -> (
        &mut movement::PendingPathRequestQueue,
        &mut Vec<movement::FailedPathRequest>,
        &SequenceManager,
    ) {
        let Self {
            pending_path_requests,
            failed_path_requests,
            sequence_manager,
            ..
        } = self;
        (
            pending_path_requests,
            failed_path_requests,
            sequence_manager,
        )
    }

    /// Atomically install preflighted legacy path queues without exposing the
    /// scheduler's mutable queue fields outside the engine module.
    pub(crate) fn install_legacy_path_schedule(
        &mut self,
        pending: movement::PendingPathRequestQueue,
        failed: Vec<movement::FailedPathRequest>,
    ) {
        self.pending_path_requests = pending;
        self.failed_path_requests = failed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_runtime_starts_with_empty_barrier_queues() {
        let mut orders = OrderRuntime::new();

        assert_eq!(orders.next_order_id, 1);
        assert_eq!(orders.allocate_order_id().get(), 1);
        assert_eq!(orders.next_order_id, 2);
        assert!(orders.failed_path_requests.is_empty());
        assert!(orders.timer_elements.is_empty());
    }
}
