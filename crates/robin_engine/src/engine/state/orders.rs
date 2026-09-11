use crate::element::EntityId;
use crate::messenger::Messenger;
use crate::sequence::SequenceManager;

use super::super::{PendingScrollAmulet, TimerEntry, movement};

/// Deterministic scheduled gameplay work and its existing drain barriers.
///
/// Owning these values together does not make their effects asynchronous:
/// every queue is still drained at its pre-existing point in the ten-phase
/// tick, and sequence/script callbacks remain same-call operations.
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

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedOrderRuntime {
    next_order_id: u32,

    messenger: Messenger,

    pending_move_requests: Vec<(EntityId, crate::order::AiOrderIntent)>,

    pending_path_requests: movement::PendingPathRequestQueue,

    failed_path_requests: Vec<movement::FailedPathRequest>,

    timer_elements: Vec<TimerEntry>,

    sequence_manager: crate::sequence::PersistedSequenceManager,

    pending_reinforcements: Vec<Option<EntityId>>,

    pending_scroll_amulets: Vec<PendingScrollAmulet>,

    pending_hero_speeches: Vec<(EntityId, u16)>,

    pending_hades_kills: Vec<EntityId>,

    pending_concussion_side_effects: Vec<(EntityId, crate::combat::ConcussionOutcome)>,
}

impl PersistedOrderRuntime {
    pub(crate) fn capture(value: &OrderRuntime) -> Self {
        let OrderRuntime {
            next_order_id: _,
            messenger: _,
            pending_move_requests: _,
            pending_path_requests: _,
            failed_path_requests: _,
            timer_elements: _,
            sequence_manager: _,
            pending_reinforcements: _,
            pending_scroll_amulets: _,
            pending_hero_speeches: _,
            pending_hades_kills: _,
            pending_concussion_side_effects: _,
        } = value;
        Self {
            next_order_id: value.next_order_id,
            messenger: value.messenger.clone(),
            pending_move_requests: value.pending_move_requests.clone(),
            pending_path_requests: value.pending_path_requests.clone(),
            failed_path_requests: value.failed_path_requests.clone(),
            timer_elements: value.timer_elements.clone(),
            sequence_manager: crate::sequence::PersistedSequenceManager::capture(
                &value.sequence_manager,
            ),
            pending_reinforcements: value.pending_reinforcements.clone(),
            pending_scroll_amulets: value.pending_scroll_amulets.clone(),
            pending_hero_speeches: value.pending_hero_speeches.clone(),
            pending_hades_kills: value.pending_hades_kills.clone(),
            pending_concussion_side_effects: value.pending_concussion_side_effects.clone(),
        }
    }

    pub(crate) fn into_runtime(self) -> OrderRuntime {
        OrderRuntime {
            next_order_id: self.next_order_id,
            messenger: self.messenger,
            pending_move_requests: self.pending_move_requests,
            pending_path_requests: self.pending_path_requests,
            failed_path_requests: self.failed_path_requests,
            timer_elements: self.timer_elements,
            sequence_manager: self.sequence_manager.into_runtime(),
            pending_reinforcements: self.pending_reinforcements,
            pending_scroll_amulets: self.pending_scroll_amulets,
            pending_hero_speeches: self.pending_hero_speeches,
            pending_hades_kills: self.pending_hades_kills,
            pending_concussion_side_effects: self.pending_concussion_side_effects,
        }
    }
}

impl OrderRuntime {
    /// Cancel movement involving a retired actor. The path queue owns its
    /// logical-head timing; do not rebuild it or bypass its completion slot.
    pub(crate) fn remove_entity(&mut self, id: EntityId) {
        self.failed_path_requests
            .retain(|request| !request.request.references_entity(id));
        self.pending_move_requests.retain(|(owner, intent)| {
            *owner != id && intent.antagonist != Some(id) && intent.target_actor != Some(id.index())
        });
        self.pending_path_requests.remove_entity(id);
    }

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

    /// Validate invariants that must survive queueing and snapshot restore.
    ///
    /// The pending-move queue deliberately permits several entries per owner:
    /// one AI decision can issue two movement requests, each
    /// of which launches its own sequence
    /// in the original game. Both survive
    /// until the sequence-manager hourglass instructs them in launch order.
    pub(crate) fn validate_invariants(&self) -> Result<(), String> {
        for (owner, intent) in &self.pending_move_requests {
            intent
                .validate_queued_move_topology()
                .map_err(|detail| format!("pending AI move for {owner:?}: {detail}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_messenger_contains_only_the_live_queue() {
        use crate::messenger::{Message, MessageType, SimpleMessage};

        let mut orders = OrderRuntime::new();
        let message = Message::new(MessageType::Simple(SimpleMessage::Pause));
        orders.messenger.send(message.clone());
        let json = serde_json::to_value(PersistedOrderRuntime::capture(&orders)).unwrap();
        assert_eq!(
            json["messenger"],
            serde_json::json!({ "queue": [message.clone()] })
        );
        let persisted: PersistedOrderRuntime = serde_json::from_value(json).unwrap();
        let mut restored = persisted.into_runtime();
        let mut snapshot: Messenger = bitcode::decode(&bitcode::encode(&orders.messenger)).unwrap();
        for messenger in [&mut restored.messenger, &mut snapshot] {
            assert_eq!(
                robin_util::state_hash::compute(messenger),
                robin_util::state_hash::compute(&orders.messenger)
            );
            assert_eq!(messenger.poll(), Some(message.clone()));
            assert_eq!(messenger.poll(), None);
        }
    }

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

    /// One AI decision can queue two movement requests for the same actor —
    /// Swordfight observation reconsideration falls through from its defensive
    /// step-back into the attack block without returning
    /// in the original game. Both
    /// Sequence launches survive in the original game, so the queue must accept
    /// repeated owners instead of collapsing them to the last intent.
    #[test]
    fn pending_move_queue_accepts_two_intents_from_one_think() {
        let mut orders = OrderRuntime::new();
        let owner = EntityId::new(7, crate::element::EntityIdKind::Pc);
        let mut intent =
            crate::order::AiOrderIntent::new(crate::order::OrderType::WalkingUpright, 10.0, 20.0);
        intent.source_position = Some(crate::coordinates::MapPoint::new(1.0, 2.0));
        intent.source_layer = Some(0);
        intent.raw_source_layer = Some(0);

        orders.pending_move_requests.push((owner, intent.clone()));
        orders.pending_move_requests.push((owner, intent));

        assert!(orders.validate_invariants().is_ok());
        assert_eq!(orders.pending_move_requests.len(), 2);
    }

    #[test]
    fn pending_move_queue_rejects_missing_call_time_topology() {
        let mut orders = OrderRuntime::new();
        let owner = EntityId::new(7, crate::element::EntityIdKind::Pc);
        orders.pending_move_requests.push((
            owner,
            crate::order::AiOrderIntent::new(crate::order::OrderType::WalkingUpright, 10.0, 20.0),
        ));

        let error = orders.validate_invariants().unwrap_err();
        assert!(error.contains("call-time source position"), "{error}");
    }

    #[test]
    fn pending_move_queue_rejects_inconsistent_exact_sector_identity() {
        let mut orders = OrderRuntime::new();
        let owner = EntityId::new(7, crate::element::EntityIdKind::Pc);
        let mut intent =
            crate::order::AiOrderIntent::new(crate::order::OrderType::WalkingUpright, 10.0, 20.0);
        intent.source_position = Some(crate::coordinates::MapPoint::new(1.0, 2.0));
        intent.source_layer = Some(0);
        intent.raw_source_layer = Some(0);
        intent.source_sector = crate::position_interface::SectorHandle::new(3).map(|sector| {
            sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(3).unwrap())
        });
        intent.source_sector_index = crate::fast_find_grid::SectorIndex::new(4);
        orders.pending_move_requests.push((owner, intent));

        let error = orders.validate_invariants().unwrap_err();
        assert!(error.contains("source sector identity"), "{error}");
    }
}
