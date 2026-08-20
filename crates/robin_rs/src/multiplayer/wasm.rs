//! WebAssembly (browser) multiplayer stub.
//!
//! The multiplayer transport is iroh-only.  iroh's browser support
//! (relay-over-WebSocket) has not been wired into this build yet, so
//! wasm clients currently cannot join multiplayer sessions.
//!
//! TODO: bring browser multiplayer back on top of iroh's wasm
//! support (`wasm32-unknown-unknown` + relay-only connectivity),
//! mirroring [`super::native`]'s `connect_client` surface.

use super::{NetEvent, NetOutbound};
use robin_engine::player_command::PlayerId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

/// Browser-side handle to an active client connection.  Never
/// constructed while the iroh wasm transport is missing; exists so
/// [`super::MultiplayerRuntime`] compiles on wasm.
pub struct ClientHandle {
    pub assigned_seat: Rc<RefCell<Option<PlayerId>>>,
    pub mission_seed: Rc<RefCell<Option<u64>>>,
    pub mission_sim_config: Rc<RefCell<Option<robin_engine::engine::SimConfig>>>,
    pub mission_id: Rc<RefCell<Option<String>>>,
}

impl ClientHandle {
    pub fn mission_seed(&self) -> Option<u64> {
        *self.mission_seed.borrow()
    }

    pub fn mission_sim_config(&self) -> Option<robin_engine::engine::SimConfig> {
        *self.mission_sim_config.borrow()
    }

    pub fn mission_id(&self) -> Option<String> {
        self.mission_id.borrow().clone()
    }

    pub fn shutdown(&mut self) {}
}

/// Browser multiplayer is unavailable until iroh's wasm support is
/// wired in; always errors.
pub fn connect_client(
    addr: impl AsRef<str>,
    _nickname: String,
    _incoming_tx: Sender<NetEvent>,
    _outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    Err(std::io::Error::other(format!(
        "multiplayer: browser builds cannot connect to `{}` yet — the iroh transport is native-only for now",
        addr.as_ref()
    )))
}
