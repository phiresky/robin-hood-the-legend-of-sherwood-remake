use serde::{Deserialize, Serialize};

use crate::{
    element::EntityId,
    engine::SeatState,
    macro_store::{AutoQueueStore, MacroStore},
    profiles::Action,
    tactical_control::TacticalControlState,
};

/// Deterministic per-player selection, input-mode, and quick-action state.
#[derive(
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct PlayerRuntime {
    pub(crate) seats: Vec<SeatState>,
    pub(crate) macro_store: MacroStore,
    /// Post-port Shift-click work. Kept separate from Original's three
    /// manual QA slots so queue advancement can never consume a saved macro.
    ///
    /// This layout starts at SAVE54/NET18/REPLAY12. Current structured
    /// snapshots must carry it explicitly; older Rust snapshots are rejected.
    pub(crate) auto_queues: AutoQueueStore,
    pub(crate) user_locked: bool,
    /// Original `RHMessenger::mbLockView`, independently serialized from the
    /// engine's camera-follow locker.
    pub(crate) view_locked: bool,
    pub(crate) selection_before_user_lock: Vec<EntityId>,
    pub(crate) qa_recording_for: Vec<EntityId>,
    pub(crate) qa_recording_slot: u8,
    pub(crate) action_before_recording_macro: Action,
    /// PCs whose Shift-click queue is waiting for its currently dispatched
    /// action (or pre-existing live work) to finish.
    pub(crate) auto_queue_active: Vec<EntityId>,
    /// High-level command state for any actor exposing `TacticalOrders`.
    /// The serialized name preserves existing saves and replay snapshots.
    #[serde(rename = "allied")]
    pub(crate) tactical: TacticalControlState,
}

impl PlayerRuntime {
    pub(crate) fn new() -> Self {
        Self {
            seats: vec![SeatState::default()],
            macro_store: MacroStore::new(),
            auto_queues: AutoQueueStore::default(),
            user_locked: false,
            view_locked: false,
            selection_before_user_lock: Vec::new(),
            qa_recording_for: Vec::new(),
            qa_recording_slot: 0,
            action_before_recording_macro: Action::NoAction,
            auto_queue_active: Vec::new(),
            tactical: TacticalControlState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::MapPoint;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};

    #[test]
    fn new_players_has_canonical_seat_zero_and_no_recording() {
        let players = PlayerRuntime::new();

        assert_eq!(players.seats.len(), 1);
        assert!(players.seats[0].selection.is_empty());
        assert!(!players.user_locked);
        assert!(!players.view_locked);
        assert!(players.qa_recording_for.is_empty());
        assert_eq!(players.qa_recording_slot, 0);
        assert_eq!(players.action_before_recording_macro, Action::NoAction);
    }

    #[test]
    fn queue_free_structured_fixture_rejects_missing_automatic_queue() {
        let encoded = serde_json::to_value(PlayerRuntime::new()).expect("serialize players");
        let mut legacy = encoded
            .as_object()
            .expect("PlayerRuntime is a JSON object")
            .clone();
        legacy.remove("auto_queues");

        let error = match serde_json::from_value::<PlayerRuntime>(legacy.into()) {
            Ok(_) => panic!("current player state requires its automatic queue"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("missing field `auto_queues`"));
    }

    #[test]
    fn automatic_queue_json_and_bitcode_roundtrip_and_participates_in_player_state_hash() {
        let pc = EntityId::Pc(crate::entity_id::PcId(1));
        let mut queued = PlayerRuntime::new();
        queued.auto_queues.push(
            pc,
            QuickActionStep {
                action: Action::Bow,
                position: MapPoint::new(10.0, 20.0),
                replay: QaReplayCommand::Move {
                    destination: MapPoint::new(10.0, 20.0),
                    running: false,
                    route: crate::macro_store::RecordedQaMoveRoute {
                        goal_sector: crate::sector::SectorNumber::new(1),
                        goal_sector_index: crate::fast_find_grid::SectorIndex::new(0)
                            .expect("valid test sector index"),
                        goal_layer: 0,
                    },
                },
            },
        );
        queued.auto_queue_active.push(pc);
        let encoded = serde_json::to_string(&queued).expect("serialize queued player runtime");
        let decoded: PlayerRuntime =
            serde_json::from_str(&encoded).expect("deserialize queued player runtime");

        assert_eq!(decoded.auto_queues.len(pc), 1);
        assert_eq!(decoded.auto_queue_active, vec![pc]);
        assert_eq!(
            robin_util::state_hash::compute(&decoded),
            robin_util::state_hash::compute(&queued)
        );
        assert_ne!(
            robin_util::state_hash::compute(&queued),
            robin_util::state_hash::compute(&PlayerRuntime::new())
        );

        let bytes = bitcode::encode(&queued);
        let binary: PlayerRuntime =
            bitcode::decode(&bytes).expect("decode queued multiplayer snapshot state");
        assert_eq!(binary.auto_queues.len(pc), 1);
        assert_eq!(binary.auto_queue_active, vec![pc]);
        assert_eq!(
            robin_util::state_hash::compute(&binary),
            robin_util::state_hash::compute(&queued)
        );
    }
}
