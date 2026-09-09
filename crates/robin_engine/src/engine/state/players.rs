use serde::{Deserialize, Serialize};

use crate::{
    element::EntityId,
    engine::SeatState,
    fog_of_war::FogOfWarState,
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
    /// Original-game messenger view lock, independently serialized from the
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
    /// Shared allied sight and temporary hostile intelligence. This never
    /// mutates the original game's permanent blipped identity bit.
    pub(crate) fog_of_war: FogOfWarState,
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedPlayerRuntime {
    seats: Vec<SeatState>,

    macro_store: MacroStore,

    auto_queues: AutoQueueStore,

    user_locked: bool,

    view_locked: bool,

    selection_before_user_lock: Vec<EntityId>,

    qa_recording_for: Vec<EntityId>,

    qa_recording_slot: u8,

    action_before_recording_macro: Action,

    auto_queue_active: Vec<EntityId>,
    #[serde(rename = "allied")]
    tactical: TacticalControlState,

    fog_of_war: crate::fog_of_war::PersistedFogOfWarState,
}

impl PersistedPlayerRuntime {
    pub(crate) fn capture(value: &PlayerRuntime) -> Self {
        let PlayerRuntime {
            seats: _,
            macro_store: _,
            auto_queues: _,
            user_locked: _,
            view_locked: _,
            selection_before_user_lock: _,
            qa_recording_for: _,
            qa_recording_slot: _,
            action_before_recording_macro: _,
            auto_queue_active: _,
            tactical: _,
            fog_of_war: _,
        } = value;
        Self {
            seats: value.seats.clone(),
            macro_store: value.macro_store.clone(),
            auto_queues: value.auto_queues.clone(),
            user_locked: value.user_locked,
            view_locked: value.view_locked,
            selection_before_user_lock: value.selection_before_user_lock.clone(),
            qa_recording_for: value.qa_recording_for.clone(),
            qa_recording_slot: value.qa_recording_slot,
            action_before_recording_macro: value.action_before_recording_macro,
            auto_queue_active: value.auto_queue_active.clone(),
            tactical: value.tactical.clone(),
            fog_of_war: crate::fog_of_war::PersistedFogOfWarState::capture(&value.fog_of_war),
        }
    }

    pub(crate) fn into_runtime(self) -> PlayerRuntime {
        PlayerRuntime {
            seats: self.seats,
            macro_store: self.macro_store,
            auto_queues: self.auto_queues,
            user_locked: self.user_locked,
            view_locked: self.view_locked,
            selection_before_user_lock: self.selection_before_user_lock,
            qa_recording_for: self.qa_recording_for,
            qa_recording_slot: self.qa_recording_slot,
            action_before_recording_macro: self.action_before_recording_macro,
            auto_queue_active: self.auto_queue_active,
            tactical: self.tactical,
            fog_of_war: self.fog_of_war.into_runtime(),
        }
    }
}

impl PlayerRuntime {
    pub(crate) fn remove_entity(&mut self, id: crate::element::EntityId) {
        for seat in &mut self.seats {
            seat.remove_entity(id);
        }
        self.selection_before_user_lock.retain(|&owner| owner != id);
        self.qa_recording_for.retain(|&owner| owner != id);
        self.auto_queue_active.retain(|&owner| owner != id);
        // Saved macros and historical fog observations are identities, not
        // live selections; they remain available for their normal consumers.
    }

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
            fog_of_war: FogOfWarState::default(),
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
    fn seat_fixture_rejects_missing_planned_shield_prompt() {
        let mut encoded = serde_json::to_value(PlayerRuntime::new()).expect("serialize players");
        encoded
            .get_mut("seats")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|seats| seats.first_mut())
            .and_then(serde_json::Value::as_object_mut)
            .expect("seat zero is a JSON object")
            .remove("planned_shield_target");

        let error = match serde_json::from_value::<PlayerRuntime>(encoded) {
            Ok(_) => panic!("current seat state requires its planned shield prompt"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("missing field `planned_shield_target`")
        );
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
