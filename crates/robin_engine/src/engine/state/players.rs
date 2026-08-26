use serde::{Deserialize, Serialize};

use crate::{
    allied_control::AlliedControlState, element::EntityId, engine::SeatState,
    macro_store::MacroStore, profiles::Action,
};

/// Deterministic per-player selection, input-mode, and quick-action state.
#[derive(Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct PlayerRuntime {
    pub(crate) seats: Vec<SeatState>,
    pub(crate) macro_store: MacroStore,
    pub(crate) user_locked: bool,
    /// Original `RHMessenger::mbLockView`, independently serialized from the
    /// engine's camera-follow locker.
    #[serde(default)]
    pub(crate) view_locked: bool,
    pub(crate) selection_before_user_lock: Vec<EntityId>,
    pub(crate) qa_recording_for: Vec<EntityId>,
    pub(crate) qa_recording_slot: u8,
    pub(crate) action_before_recording_macro: Action,
    /// PCs whose Shift-click queue is waiting for its currently dispatched
    /// action (or pre-existing live work) to finish.
    #[serde(default)]
    pub(crate) auto_queue_active: Vec<EntityId>,
    #[serde(default)]
    pub(crate) allied: AlliedControlState,
}

impl PlayerRuntime {
    pub(crate) fn new() -> Self {
        Self {
            seats: vec![SeatState::default()],
            macro_store: MacroStore::new(),
            user_locked: false,
            view_locked: false,
            selection_before_user_lock: Vec::new(),
            qa_recording_for: Vec::new(),
            qa_recording_slot: 0,
            action_before_recording_macro: Action::NoAction,
            auto_queue_active: Vec::new(),
            allied: AlliedControlState::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
