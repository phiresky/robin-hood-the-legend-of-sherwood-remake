//! [`EngineManager`] — owner of the simulation engine and its
//! immediate rollback / lockstep bookkeeping.
//!
//! `Engine` is the deterministic kernel; `EngineManager` is the host-
//! side stuff the per-frame loop needs to drive that engine in lockstep
//! with one or more peers:
//!
//! - The engine itself (field [`Self::engine`]).
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
//! while the timeline owner reads its frame cursor — no nested-borrow
//! gymnastics required. EngineManager deliberately does not own a frame
//! cursor: host/network/replay frame identity belongs to the driver timeline,
//! while `Engine::frame_counter` counts only executed simulation ticks.
//! EngineManager never applies player input itself;
//! drained inputs must enter `Engine::advance_frame` with the rest of the
//! authoritative frame transaction.

use crate::engine::Engine;

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
}

impl EngineManager {
    /// Wrap a freshly-constructed deterministic engine. Host seat and
    /// transport ownership live together on `Host::transport`.
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}
