//! Per-seat (per-player) sim-tracked state.
//!
//! A "seat" is one input source — a player at the host, a remote peer,
//! or a headless observer.  Seats are numbered by **join order** as
//! [`crate::player_command::PlayerId`] values: seat 0 is `HOST` (the
//! host or first-joined peer), seats 1+ are subsequent joiners.  The
//! numbering is sim-side and identical across every machine in the
//! session, so a recording produced on peer-2 is byte-for-byte the
//! same as one produced on the host.
//!
//! Each seat has its own selection and hotgroups. Local viewport state
//! lives host-side; the engine only keeps the shared script/director
//! camera. The fields here used to live as flat members on
//! [`super::EngineInner`]; folding them into a vector of
//! [`SeatState`] lets the engine support multiple simultaneous players
//! without changing the dispatch surface for every selection command.

use crate::element::EntityId;
use crate::profiles::Action;
use serde::{Deserialize, Serialize};

/// Sim-tracked state owned by one player seat.
#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SeatState {
    /// Whether the seat currently has a player attached.  Cleared by
    /// [`crate::player_command::PlayerCommand::DisconnectSeat`] but
    /// the rest of `SeatState` (selection, hotgroups) is preserved
    /// so the seat's PCs stay where they were left.  A subsequent
    /// `ConnectSeat` flips this back without resetting selection,
    /// supporting drop-in/drop-out.
    ///
    /// The host seat (`PlayerId::HOST`, index 0) is implicitly
    /// connected from level start — single-player and headful host
    /// setups never emit a `ConnectSeat` for it.  Use
    /// [`SeatState::is_active`] for the "connected or host seat"
    /// query.
    pub connected: bool,
    /// Player nickname for the portrait "controlled by" overlay.
    /// Empty for the implicit host seat in single-player.  Recorded
    /// in replays so a bug-report replay reproduces the labels.
    pub nickname: String,
    /// PCs this seat currently has selected. Multiple seats may select
    /// the same PC simultaneously (drop-in/drop-out + co-op control).
    pub selection: Vec<EntityId>,
    /// Original `RHMessenger::muwAction`: the action currently armed by this
    /// player's input controller. This is deliberately independent of a PC's
    /// remembered `current_action`; `MSG_SELECT_ACTION_SIMPLE` clears only
    /// this value when an action becomes unavailable.
    #[serde(default)]
    pub selected_action: Action,
    /// Original `RHMessenger::muwActionBeforeControl`, used to restore the
    /// globally armed action when Ctrl is released.
    #[serde(default)]
    pub action_before_control: Action,
    /// Quick-select groups (Ctrl+1..9 to assign, 1..9 to recall).
    /// Index 0 = group 1, index 8 = group 9.
    pub quick_select_groups: [Vec<EntityId>; 9],

    /// Whether the cutscene/director camera is locked to follow [`follow_element`]
    /// (locker / follow-cam).
    pub locker_active: bool,

    /// Entity the cutscene/director camera follows while [`locker_active`] is true.
    pub follow_element: Option<EntityId>,

    /// Whether the alt-lock toggle (caps-lock-style "always show
    /// vision cones / target-arrows") is on for this seat.  Each
    /// player keeps their own alt-lock state — flipping it on one
    /// machine doesn't toggle on the other.
    pub is_lock_alt: bool,
}

impl SeatState {
    /// True if this seat currently has an attached player driving it.
    /// The host seat (index 0) is always considered active even
    /// without an explicit `ConnectSeat`, so single-player and
    /// headful-host setups don't need to bootstrap the lifecycle.
    pub fn is_active(&self, seat_index: usize) -> bool {
        seat_index == 0 || self.connected
    }
}
