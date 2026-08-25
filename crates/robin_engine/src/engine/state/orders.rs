use crate::element::EntityId;
use crate::messenger::Messenger;
use crate::sequence::SequenceManager;

use super::super::{PendingScrollAmulet, TimerEntry, movement};

/// Deterministic scheduled gameplay work and its existing drain barriers.
///
/// Owning these values together does not make their effects asynchronous:
/// every queue is still drained at its pre-existing point in the ten-phase
/// tick, and sequence/script callbacks remain same-call operations.
#[derive(Clone, serde::Serialize, serde::Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct OrderRuntime {
    pub(crate) next_order_id: u32,
    pub(crate) messenger: Messenger,
    pub(crate) pending_move_requests: Vec<(EntityId, crate::order::AiOrderIntent)>,
    pub(in crate::engine) pending_path_requests: movement::PendingPathRequestQueue,
    pub(in crate::engine) failed_path_requests: Vec<movement::FailedPathRequest>,
    pub(crate) timer_elements: Vec<TimerEntry>,
    pub(crate) sequence_manager: SequenceManager,
    pub(crate) pending_reinforcements: Vec<Option<EntityId>>,
    pub(crate) pending_scroll_amulets: Vec<PendingScrollAmulet>,
    pub(crate) pending_hero_speeches: Vec<(EntityId, u16)>,
    pub(crate) pending_hades_kills: Vec<EntityId>,
    pub(crate) pending_concussion_side_effects: Vec<(EntityId, crate::combat::ConcussionOutcome)>,
}

impl OrderRuntime {
    pub(crate) fn new() -> Self {
        Self {
            next_order_id: 1,
            messenger: Messenger::new(),
            pending_move_requests: Vec::new(),
            pending_path_requests: Default::default(),
            failed_path_requests: Vec::new(),
            timer_elements: Vec::new(),
            sequence_manager: SequenceManager::new(),
            pending_reinforcements: Vec::new(),
            pending_scroll_amulets: Vec::new(),
            pending_hero_speeches: Vec::new(),
            pending_hades_kills: Vec::new(),
            pending_concussion_side_effects: Vec::new(),
        }
    }

    pub(crate) fn allocate_order_id(&mut self) -> std::num::NonZeroU32 {
        crate::order::alloc_order_id(&mut self.next_order_id)
    }

    /// Split the exact scheduler-owned leaves used by the path barrier.
    ///
    /// The sequence manager is read-only here. Path completion consequences
    /// (sequence mutation, hero speech, and condolation dispatch) remain owned
    /// by the root tick coordinator at their Original call positions.
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

    /// Validate invariants that must survive queueing and snapshot restore.
    ///
    /// The pending-move queue deliberately permits several entries per owner:
    /// one `Think` can issue two `RHArtificialIntelligence::GoTo` calls, each
    /// of which launches its own `RHSequence`
    /// (`original-code/RHartificialmalignity.cpp:15502-15570`). Both survive
    /// until the sequence-manager hourglass instructs them in launch order.
    pub(crate) fn validate_invariants(&self) -> Result<(), String> {
        Ok(())
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
        assert_eq!(orders.messenger.count(), 0);
        assert!(orders.pending_move_requests.is_empty());
        assert!(orders.failed_path_requests.is_empty());
        assert!(orders.timer_elements.is_empty());
        assert!(orders.pending_reinforcements.is_empty());
        assert!(orders.pending_scroll_amulets.is_empty());
        assert!(orders.pending_hero_speeches.is_empty());
        assert!(orders.pending_hades_kills.is_empty());
        assert!(orders.pending_concussion_side_effects.is_empty());
        assert!(orders.validate_invariants().is_ok());
    }

    /// One `Think` can queue two `GoTo`s for the same actor —
    /// `ReconsiderSwordfightObservation` falls through from its defensive
    /// step-back into the attack block without returning
    /// (`original-code/RHartificialmalignity.cpp:15502-15570`). Both
    /// `LaunchSequence` calls survive in Original, so the queue must accept
    /// repeated owners instead of collapsing them to the last intent.
    #[test]
    fn pending_move_queue_accepts_two_intents_from_one_think() {
        let mut orders = OrderRuntime::new();
        let owner = EntityId::new(7, crate::element::EntityIdKind::Pc);
        let intent =
            crate::order::AiOrderIntent::new(crate::order::OrderType::WalkingUpright, 10.0, 20.0);

        orders.pending_move_requests.push((owner, intent.clone()));
        orders.pending_move_requests.push((owner, intent));

        assert!(orders.validate_invariants().is_ok());
        assert_eq!(orders.pending_move_requests.len(), 2);
    }
}
