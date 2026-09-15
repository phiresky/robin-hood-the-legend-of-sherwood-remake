//! Multiplayer transport stand-ins for builds without the `multiplayer`
//! feature.
//!
//! Mounted by `multiplayer.rs` as `transport` in place of `enabled.rs`, with
//! the same shared names, so callers need no feature gates. No runtime can
//! exist. Local sessions use [`super::current_epoch_ms`] for their scheduling
//! clock.

#[cfg(test)]
pub(crate) use robin_engine::multiplayer::MultiplayerSessionId;

/// Uninhabited: without the feature no transport can be started, so the
/// channel bundle never holds a runtime.
pub enum MultiplayerRuntime {}

impl MultiplayerRuntime {
    /// Stop the transport now. Calling this more than once is harmless.
    pub fn shutdown(&mut self) {
        match *self {}
    }

    pub(super) fn preserve_session_for_next_mission(&mut self) {
        match *self {}
    }
}

/// Campaign lifetime token. Without a transport there is no campaign state
/// to carry between missions.
#[derive(Default)]
pub struct MultiplayerCampaignSession;
