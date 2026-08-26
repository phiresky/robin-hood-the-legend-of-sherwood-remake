//! [`EngineManager`] — owner of the simulation engine and its
//! immediate rollback / lockstep bookkeeping.
//!
//! `Engine` is the deterministic kernel; `EngineManager` is the host-
//! side stuff the per-frame loop needs to drive that engine in lockstep
//! with one or more peers:
//!
//! - The engine itself (field [`Self::engine`]).
//! - The current `sim_frame` (field [`Self::sim_frame`]) — single
//!   source of truth for "what frame is the engine about to tick".
//! - The future-input queue ([`Self::pending_inputs`], mediated by
//!   [`Self::admit_input_at`] / [`Self::take_due_inputs`]) — peer
//!   inputs stamped for a frame the local sim hasn't reached yet.
//! - Which seat the local player owns ([`Self::local_seat`]).
//!
//! The wire transport itself (`NetChannels`) lives on `Host`, not on
//! the manager — moving it would cascade into the entire transport
//! setup and dispatch path, and the manager doesn't need it for any
//! of its own methods.  The host's [`crate::game_session::dispatch_local_command`]
//! reads the host transport to decide between "send over wire" and "stage in
//! the current frame".
//!
//! Borrowing pattern: all fields are `pub`.  Rust allows simultaneous
//! disjoint-field borrows, so a helper can take `&mut manager.engine`
//! while the same scope reads `manager.sim_frame` — no nested-borrow
//! gymnastics required. EngineManager never applies player input itself;
//! drained inputs must enter `Engine::advance_frame` with the rest of the
//! authoritative frame transaction.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::engine::Engine;
use crate::player_command::{PlayerId, PlayerInput};

/// A peer-or-self input whose `target_frame` is in the past relative
/// to the current `sim_frame`.  Returned by
/// [`EngineManager::admit_input_at`] so the host loop can route the
/// input through its rollback machinery (splice into the per-frame
/// command log, rewind to `target_frame`, replay forward).
#[derive(Debug, Clone)]
pub struct LateInput {
    pub target_frame: u32,
    pub input: PlayerInput,
}

/// Owner of the simulation engine plus per-frame rollback state.
/// See module docs.
pub struct EngineManager {
    /// The simulation engine.  Mutate directly via
    /// `&mut manager.engine` for level-load / dev-rewind paths; per-frame
    /// mutations must go through `Engine::advance_frame`, and player commands
    /// should enter via the host
    /// helper `dispatch_local_command` (which routes over the wire in
    /// MP) instead.
    pub engine: Engine,
    /// The seat the local player owns.  Stamped onto every locally-
    /// produced [`PlayerInput`].  In single-player this is
    /// [`PlayerId::HOST`].
    pub local_seat: PlayerId,
    /// Host timeline cursor. Drivers advance it only after committing the
    /// corresponding authoritative frame.
    pub sim_frame: u32,
    /// Inputs scheduled for a future frame.  Drained by
    /// [`Self::take_due_inputs`] when `sim_frame` reaches the keyed
    /// frame.  Public so the snapshot-adopt path can clear stale
    /// entries directly.
    pub pending_inputs: BTreeMap<u32, Vec<PlayerInput>>,
}

impl EngineManager {
    /// Wrap a freshly-constructed engine.  `local_seat` should be
    /// [`PlayerId::HOST`] for the host / single-player; clients set it
    /// to whatever seat the server assigns through the wire handshake.
    pub fn new(engine: Engine, local_seat: PlayerId) -> Self {
        Self {
            engine,
            local_seat,
            sim_frame: 0,
            pending_inputs: BTreeMap::new(),
        }
    }

    /// Admit a frame-stamped input into the lockstep queue.
    ///
    /// - `target >= sim_frame` → queued for frame-transaction admission.
    /// - `target < sim_frame`  → returned as [`LateInput`] for the
    ///   caller to route through its rollback buffer (splice into the
    ///   per-frame command log, rewind to `target`, replay forward).
    pub fn admit_input_at(
        &mut self,
        target_frame: u32,
        input: PlayerInput,
    ) -> Result<(), LateInput> {
        match target_frame.cmp(&self.sim_frame) {
            Ordering::Greater | Ordering::Equal => {
                self.pending_inputs
                    .entry(target_frame)
                    .or_default()
                    .push(input);
                Ok(())
            }
            Ordering::Less => Err(LateInput {
                target_frame,
                input,
            }),
        }
    }

    /// Drain inputs scheduled for the current `sim_frame`. The caller must
    /// append them to that frame's `SimulationFrameInput`; this method never
    /// mutates the engine.
    pub fn take_due_inputs(&mut self) -> Vec<PlayerInput> {
        self.pending_inputs
            .remove(&self.sim_frame)
            .unwrap_or_default()
    }

    /// Force `sim_frame` to a specific value.  Used when adopting an
    /// authoritative initial-state snapshot from the host so the
    /// joining client's clock aligns with the host's.
    pub fn set_sim_frame(&mut self, frame: u32) {
        self.sim_frame = frame;
    }

    /// Discard any pending inputs older than `frame`.  Used after a
    /// snapshot adopt (everything before that frame is baked in).
    pub fn drop_pending_inputs_before(&mut self, frame: u32) {
        self.pending_inputs.retain(|&f, _| f >= frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::Campaign;
    use crate::engine::LevelAssets;
    use crate::player_command::PlayerCommand;

    fn make_engine() -> Engine {
        let mut assets = LevelAssets::default();
        Engine::new_for_test(640.0, 480.0, Campaign::default(), &mut assets).expect("engine")
    }

    #[test]
    fn admit_input_at_future_queues() {
        let mut mgr = EngineManager::new(make_engine(), PlayerId::HOST);
        let input = PlayerInput::new(PlayerId(1), PlayerCommand::CrouchDown);
        let r = mgr.admit_input_at(10, input);
        assert!(r.is_ok());
        assert_eq!(mgr.pending_inputs.get(&10).map(|v| v.len()), Some(1));
    }

    #[test]
    fn admit_input_at_past_returns_late() {
        let mut mgr = EngineManager::new(make_engine(), PlayerId::HOST);
        mgr.set_sim_frame(20);
        let input = PlayerInput::new(PlayerId(1), PlayerCommand::CrouchDown);
        let r = mgr.admit_input_at(10, input);
        match r {
            Err(LateInput { target_frame, .. }) => assert_eq!(target_frame, 10),
            Ok(()) => panic!("expected LateInput error"),
        }
    }

    #[test]
    fn take_due_inputs_returns_current_frame_without_mutating_engine() {
        let mut mgr = EngineManager::new(make_engine(), PlayerId::HOST);
        let input = PlayerInput::new(PlayerId(1), PlayerCommand::CrouchDown);
        mgr.admit_input_at(5, input.clone()).unwrap();
        mgr.set_sim_frame(5);
        let before = crate::replay::state_hash(&mgr.engine);
        let drained = mgr.take_due_inputs();
        assert_eq!(drained.len(), 1);
        assert!(mgr.pending_inputs.is_empty());
        assert_eq!(crate::replay::state_hash(&mgr.engine), before);
    }

    #[test]
    fn input_for_current_frame_is_staged_until_drained() {
        let mut mgr = EngineManager::new(make_engine(), PlayerId::HOST);
        mgr.admit_input_at(0, PlayerInput::new(PlayerId(1), PlayerCommand::CrouchDown))
            .unwrap();
        assert_eq!(mgr.pending_inputs.get(&0).map(Vec::len), Some(1));
    }
}
