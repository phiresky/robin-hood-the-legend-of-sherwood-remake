//! Native iroh (peer-to-peer QUIC) server / client for the
//! multiplayer transport.  Each external function spawns one OS
//! thread that owns a tokio runtime driving the iroh endpoint; the
//! game loop talks to it through [`super::NetChannels`].
//!
//! Peers are addressed by iroh endpoint id (a public key), not by
//! host:port.  Connectivity — hole punching, relay fallback, address
//! lookup — is handled entirely by iroh, so hosting needs no port
//! forwarding and no bind-address configuration.
//!
//! Each session runs over a single bidirectional QUIC stream per
//! peer, carrying length-prefixed [`NetMsg`] frames.  The joining
//! side opens the stream and sends `Hello`; the host answers
//! `Welcome` on the same stream.

mod peer_sessions;
use peer_sessions::PeerSessions;

mod server_dispatch;
#[cfg(test)]
use server_dispatch::validate_server_gameplay_outbound;
use server_dispatch::{announce_begin_sim, broadcast_input, run_server_outgoing_pump};
mod server_protocol;
use server_protocol::{PendingSnapshotTransition, ReadyBarrier, SnapshotTransitions};

#[cfg(test)]
use super::encode_msg;
use super::framing::{read_frame, write_frame};
use super::identity::{
    GAME_ALPN, bind_endpoint, bind_endpoint_with_relay, game_secret_key, parse_connect_addr,
};
use super::{
    FrameCursor, INPUT_DELAY_FRAMES, InboundFramePolicy, InitialSnapshot, MultiplayerError,
    MultiplayerSessionId, NET_PROTOCOL_VERSION, NetEvent, NetMsg, NetOutbound,
};
use crate::distributed_mod::{
    DistributedModPackage, ValidatedDistributedMod, make_distributed_mod_offer,
};
use iroh::endpoint::{RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
// Non-poisoning mutex: a panicking worker must not turn every later
// lock of the shared peer state into a second panic.
#[cfg(test)]
use super::clock::checked_epoch_ms;
use super::clock::try_current_epoch_ms;
use parking_lot::Mutex;
use robin_engine::multiplayer::{BrowserPeerAuth, NetFatal, browser_seat_proof_message};
use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);

const HANDSHAKE_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
/// Finish queued reconnect/commit frames after reader authority is detached.
const TERMINAL_WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);
const CONTENT_TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CONTENT_DECISION_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CONTENT_READINESS_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// QUIC close code used for orderly application shutdown.
const CLOSE_GRACEFUL: u32 = 0;

/// Canonical validated package a host distributes before Welcome/snapshot.
#[derive(Clone, Debug)]
pub struct HostedModContent {
    validated: ValidatedDistributedMod,
    encoded: Arc<[u8]>,
}

impl HostedModContent {
    pub fn from_encoded(encoded: Vec<u8>) -> Result<Self, MultiplayerError> {
        let validated = DistributedModPackage::decode(&encoded).map_err(|error| {
            MultiplayerError::ContentMismatch(
                format!("validate hosted distributed mod: {error}").into(),
            )
        })?;
        Ok(Self {
            validated,
            encoded: Arc::from(encoded),
        })
    }

    fn offer(
        &self,
        host_endpoint_id: String,
    ) -> Result<robin_engine::multiplayer::DistributedModOffer, MultiplayerError> {
        make_distributed_mod_offer(&self.validated, self.encoded.len() as u64, host_endpoint_id)
            .map_err(|error| {
                MultiplayerError::ContentMismatch(
                    format!("build distributed-mod offer: {error}").into(),
                )
            })
    }
}

/// Explicit campaign lifetime, independent of each mission's QUIC endpoint.
/// Transport credentials are deliberately neither persisted nor reconstructed
/// by deserialization.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MultiplayerCampaignSession {
    #[serde(skip)]
    state: Option<Arc<CampaignTransportState>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CampaignTransportState {
    #[serde(skip, default = "SecretKey::generate")]
    client_key: SecretKey,
    #[serde(skip)]
    continuation: Mutex<Option<HostSessionContinuation>>,
    #[serde(skip)]
    server_active: AtomicBool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CampaignServerLease {
    #[serde(skip)]
    state: Option<Arc<CampaignTransportState>>,
}

impl Drop for CampaignServerLease {
    fn drop(&mut self) {
        if let Some(state) = &self.state {
            state.server_active.store(false, Ordering::Release);
        }
    }
}

impl Default for MultiplayerCampaignSession {
    fn default() -> Self {
        Self {
            state: Some(Arc::new(CampaignTransportState {
                client_key: SecretKey::generate(),
                continuation: Mutex::new(None),
                server_active: AtomicBool::new(false),
            })),
        }
    }
}

impl MultiplayerCampaignSession {
    fn state(&self) -> &Arc<CampaignTransportState> {
        self.state
            .as_ref()
            .expect("decoded campaign has no live multiplayer authority")
    }

    pub(crate) fn discard_host_continuation(&self) -> Result<(), MultiplayerError> {
        let _lease = self.reserve_server()?;
        *self.state().continuation.lock() = None;
        Ok(())
    }

    fn reserve_server(&self) -> std::io::Result<CampaignServerLease> {
        self.state()
            .server_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                std::io::Error::other("campaign already owns an active mission transport")
            })?;
        Ok(CampaignServerLease {
            state: Some(Arc::clone(self.state())),
        })
    }
}

/// One-shot campaign-local handoff between the old and replacement mission
/// transports. The outer campaign loop deliberately destroys each QUIC
/// endpoint at a load/restart boundary, but the authenticated session and its
/// seat ownership must survive that implementation detail.
#[derive(Clone)]
struct HostSessionContinuation {
    host_endpoint_id: EndpointId,
    session_id: MultiplayerSessionId,
    expected_players: u32,
    owner_seats: HashMap<PeerOwner, u8>,
    relay_url: Option<iroh::RelayUrl>,
}

fn publish_host_session_continuation(
    state: &CampaignTransportState,
    continuation: HostSessionContinuation,
) {
    let mut slot = state.continuation.lock();
    if let Some(existing) = slot.as_mut()
        && existing.host_endpoint_id == continuation.host_endpoint_id
        && existing.session_id == continuation.session_id
    {
        existing.owner_seats.extend(continuation.owner_seats);
        return;
    }
    assert!(
        slot.is_none(),
        "another multiplayer continuation is pending"
    );
    *slot = Some(continuation);
}

fn pending_host_session_continuation(
    state: &CampaignTransportState,
    host_endpoint_id: EndpointId,
    expected_players: u32,
) -> Result<Option<HostSessionContinuation>, MultiplayerError> {
    let slot = state.continuation.lock();
    let Some(continuation) = slot.as_ref() else {
        return Ok(None);
    };
    if continuation.host_endpoint_id != host_endpoint_id {
        return Err(MultiplayerError::LocalState(
            "pending multiplayer continuation belongs to another host identity".into(),
        ));
    }
    if continuation.expected_players != expected_players {
        return Err(MultiplayerError::LocalState(format!(
            "continued multiplayer session expects {} players, replacement requested {expected_players}",
            continuation.expected_players
        ).into()));
    }
    Ok(slot.clone())
}

// ─── Framing ─────────────────────────────────────────────────────
// `write_frame`/`read_frame` are shared with the browser client in `framing`.

async fn read_frame_bounded_with_timeout(
    recv: &mut RecvStream,
    policy: InboundFramePolicy,
    timeout: Duration,
    phase: &'static str,
) -> Result<Option<NetMsg>, MultiplayerError> {
    tokio::time::timeout(timeout, read_frame(recv, policy))
        .await
        .map_err(|_| MultiplayerError::timeout(phase, timeout))?
}

async fn write_frame_with_timeout(
    send: &mut SendStream,
    msg: &NetMsg,
    timeout: Duration,
    phase: &'static str,
) -> Result<(), MultiplayerError> {
    tokio::time::timeout(timeout, write_frame(send, msg))
        .await
        .map_err(|_| MultiplayerError::timeout(phase, timeout))?
}

/// Bridge a std mpsc receiver (game loop side) onto a tokio unbounded
/// channel so async code can `select!` on it.  The bridge thread
/// exits when cancellation flips or either channel closes.
fn spawn_outgoing_bridge(
    name: &str,
    outgoing_rx: Receiver<NetOutbound>,
    cancellation: Arc<AtomicBool>,
) -> std::io::Result<(JoinHandle<()>, UnboundedReceiver<NetOutbound>)> {
    let (tx, rx) = unbounded_channel::<NetOutbound>();
    let handle = thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            while !cancellation.load(Ordering::Acquire) {
                let msg = match outgoing_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
        })?;
    Ok((handle, rx))
}

mod server;
pub use server::*;
mod client;
pub use client::*;

#[cfg(test)]
mod tests;
