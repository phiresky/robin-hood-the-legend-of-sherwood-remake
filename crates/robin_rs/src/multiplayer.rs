//! Multiplayer transport — WebSocket-based server / client.
//!
//! The wire-format types ([`NetMsg`], [`NetEvent`], [`NetOutbound`])
//! and protocol constants live in
//! [`robin_engine::multiplayer`] so [`robin_engine::engine_manager::EngineManager`]
//! can route mutations through the rollback-safe path. This module wraps
//! the engine channel bundle in [`NetChannels`] so the channels and their
//! platform-specific [`MultiplayerRuntime`] have one owner and one lifetime.

use robin_engine::multiplayer::NetChannels as EngineNetChannels;
#[cfg(test)]
use robin_engine::multiplayer::new_frame_cursor;
pub(crate) use robin_engine::multiplayer::{
    FrameCursor, INPUT_DELAY_FRAMES, InitialSnapshot, NET_PROTOCOL_VERSION, NetEvent, NetMsg,
    NetOutbound, STATE_HASH_INTERVAL, decode_msg, encode_msg,
};
use std::ops::Deref;
use std::sync::mpsc::{Receiver, Sender};

pub mod lobby;

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{ClientHandle, ServerHandle, connect_client, start_server};

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(target_arch = "wasm32")]
pub use wasm::{ClientHandle, connect_client};

/// Owns every worker and platform resource for one multiplayer transport.
///
/// Dropping the runtime cancels its workers, closes sockets/listeners, and
/// joins native threads. The browser implementation removes its callbacks,
/// cancels its outgoing timer, and closes the WebSocket.
///
/// Original provenance: `original-code/sblibng/SBNetwork.cpp`,
/// `SBNetwork::~SBNetwork()` closes an active session and releases the
/// DirectPlay object. This runtime preserves that resource-owning RAII
/// behavior for the port's WebSocket transport.
pub enum MultiplayerRuntime {
    #[cfg(not(target_arch = "wasm32"))]
    Server(ServerHandle),
    Client(ClientHandle),
}

#[cfg(not(target_arch = "wasm32"))]
impl From<ServerHandle> for MultiplayerRuntime {
    fn from(handle: ServerHandle) -> Self {
        Self::Server(handle)
    }
}

impl From<ClientHandle> for MultiplayerRuntime {
    fn from(handle: ClientHandle) -> Self {
        Self::Client(handle)
    }
}

impl MultiplayerRuntime {
    /// Stop the transport now. Calling this more than once is harmless.
    pub fn shutdown(&mut self) {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Server(handle) => handle.shutdown(),
            Self::Client(handle) => handle.shutdown(),
        }
    }
}

impl Drop for MultiplayerRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Game-loop channels coupled to the runtime that services them.
///
/// Field order is intentional: the engine channel senders are dropped before
/// the runtime, then runtime shutdown joins workers after their channel ends
/// have closed.
pub struct NetChannels {
    channels: EngineNetChannels,
    runtime: Option<MultiplayerRuntime>,
}

impl NetChannels {
    /// Build an unattached channel bundle. The caller must attach the runtime
    /// returned by [`start_server`] or [`connect_client`] before publishing the
    /// bundle to the game loop.
    pub fn new() -> (
        Self,
        Sender<NetEvent>,
        Receiver<NetOutbound>,
        FrameCursor,
        InitialSnapshot,
    ) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) =
            EngineNetChannels::new();
        (
            Self {
                channels,
                runtime: None,
            },
            incoming_tx,
            outgoing_rx,
            frame_cursor,
            initial_snapshot,
        )
    }

    /// Couple the channel bundle to its transport owner.
    pub fn attach_runtime(&mut self, runtime: impl Into<MultiplayerRuntime>) {
        assert!(
            self.runtime.is_none(),
            "multiplayer channels already have an attached runtime"
        );
        self.runtime = Some(runtime.into());
    }

    /// Explicitly stop and detach the transport. Drop performs the same work.
    pub fn shutdown(&mut self) {
        if let Some(mut runtime) = self.runtime.take() {
            runtime.shutdown();
        }
    }
}

impl Deref for NetChannels {
    type Target = EngineNetChannels;

    fn deref(&self) -> &Self::Target {
        &self.channels
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::multiplayer::native::{connect_client, start_server};
    use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn make_server_handle() -> (NetChannels, ServerHandle, std::net::SocketAddr) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) =
            NetChannels::new();
        let handle = start_server(
            "127.0.0.1:0",
            "host".into(),
            42,
            incoming_tx,
            outgoing_rx,
            frame_cursor,
            initial_snapshot,
            1,
        )
        .expect("start server on an ephemeral port");
        let local_addr = handle.local_addr();
        (channels, handle, local_addr)
    }

    fn start_owned_server() -> (NetChannels, std::net::SocketAddr) {
        let (mut channels, handle, local_addr) = make_server_handle();
        channels.attach_runtime(handle);
        (channels, local_addr)
    }

    #[test]
    fn dropping_owned_runtime_releases_listener() {
        let (channels, local_addr) = start_owned_server();

        drop(channels);

        let rebound = std::net::TcpListener::bind(local_addr)
            .expect("runtime drop must stop and join the listener before returning");
        drop(rebound);
    }

    #[test]
    fn dropping_runtime_joins_idle_peer_worker() {
        let (mut channels, handle, local_addr) = make_server_handle();
        let idle_peer = std::net::TcpStream::connect(local_addr).expect("connect idle peer");
        let accept_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while handle.peer_worker_count() == 0 {
            assert!(
                std::time::Instant::now() < accept_deadline,
                "server did not start the idle peer worker"
            );
            std::thread::yield_now();
        }
        channels.attach_runtime(handle);
        let (done_tx, done_rx) = channel();

        let shutdown_thread = std::thread::spawn(move || {
            drop(channels);
            done_tx.send(()).expect("report completed shutdown");
        });

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("runtime shutdown must cancel and join an idle handshake worker");
        shutdown_thread
            .join()
            .expect("runtime shutdown test worker must not panic");
        drop(idle_peer);
        let rebound = std::net::TcpListener::bind(local_addr)
            .expect("joined runtime must release its listener");
        drop(rebound);
    }

    #[test]
    fn server_client_input_roundtrip() {
        // Server side.
        let (server_in_tx, server_in_rx) = channel::<NetEvent>();
        let (_server_out_tx, server_out_rx) = channel::<NetOutbound>();
        let server_cursor = new_frame_cursor();
        let server_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let _server = start_server(
            "127.0.0.1:0",
            "host".into(),
            42,
            server_in_tx,
            server_out_rx,
            server_cursor,
            server_snapshot,
            2,
        )
        .expect("start_server");
        let addr = _server.local_addr().to_string();

        // Client side.
        let (client_in_tx, client_in_rx) = channel::<NetEvent>();
        let (client_out_tx, client_out_rx) = channel::<NetOutbound>();
        let _client = connect_client(&addr, "alice".into(), client_in_tx, client_out_rx)
            .expect("connect_client");

        let assigned = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::AssignedLocalSeat(p)) => break p,
                Ok(NetEvent::Note(_)) => continue,
                Ok(other) => panic!("unexpected pre-handshake event {other:?}"),
                Err(e) => panic!("timeout waiting for AssignedLocalSeat: {e}"),
            }
        };
        assert_eq!(assigned, PlayerId(1));

        let mut saw_join = false;
        for _ in 0..16 {
            match server_in_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(NetEvent::Input { input, .. }) => {
                    if let PlayerCommand::ConnectSeat {
                        player_id,
                        ref nickname,
                        ..
                    } = input.command
                        && player_id == PlayerId(1)
                        && nickname == "alice"
                    {
                        saw_join = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            saw_join,
            "server should have folded a ConnectSeat for the new client"
        );

        client_out_tx
            .send(NetOutbound::Input {
                origin_frame: 0,
                command: PlayerCommand::CrouchDown,
            })
            .unwrap();

        let (server_input, server_target) = loop {
            match server_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input {
                    input,
                    target_frame,
                    ..
                }) if matches!(input.command, PlayerCommand::CrouchDown) => {
                    break (input, target_frame);
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for server-side input echo: {e}"),
            }
        };
        assert_eq!(server_input.player_id, PlayerId(1));
        assert_eq!(server_target, INPUT_DELAY_FRAMES);

        let client_seen = loop {
            match client_in_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetEvent::Input { input, .. })
                    if matches!(input.command, PlayerCommand::CrouchDown) =>
                {
                    break input;
                }
                Ok(_) => continue,
                Err(e) => panic!("timeout waiting for client-side input echo: {e}"),
            }
        };
        assert_eq!(client_seen.player_id, PlayerId(1));

        let _ = (PlayerInput::new(PlayerId(0), PlayerCommand::CrouchDown),);
    }
}
