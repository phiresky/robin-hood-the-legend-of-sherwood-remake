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

use super::identity::{
    GAME_ALPN, bind_endpoint, bind_endpoint_with_relay, game_secret_key, parse_connect_addr,
};
use super::{
    FrameCursor, INPUT_DELAY_FRAMES, InboundFramePolicy, InitialSnapshot, MultiplayerSessionId,
    NET_PROTOCOL_VERSION, NetEvent, NetFrameClass, NetMsg, NetOutbound, decode_msg, encode_msg,
    net_frame_class,
};
use iroh::endpoint::{Connection, ReadExactError, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
// Non-poisoning mutex: a panicking worker must not turn every later
// lock of the shared peer state into a second panic.
use parking_lot::Mutex;
use robin_engine::multiplayer::{BrowserPeerAuth, browser_seat_proof_message};
use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// QUIC close code used for orderly application shutdown.
const CLOSE_GRACEFUL: u32 = 0;

/// One-shot process-local handoff between the old and replacement mission
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

fn host_session_continuation() -> &'static Mutex<Option<HostSessionContinuation>> {
    static SLOT: OnceLock<Mutex<Option<HostSessionContinuation>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn publish_host_session_continuation(continuation: HostSessionContinuation) {
    let mut slot = host_session_continuation().lock();
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

pub(super) fn discard_host_session_continuation() {
    *host_session_continuation().lock() = None;
}

fn take_host_session_continuation(
    host_endpoint_id: EndpointId,
    expected_players: u32,
) -> Result<Option<HostSessionContinuation>, String> {
    let mut slot = host_session_continuation().lock();
    let Some(continuation) = slot.as_ref() else {
        return Ok(None);
    };
    if continuation.host_endpoint_id != host_endpoint_id {
        return Err("pending multiplayer continuation belongs to another host identity".into());
    }
    if continuation.expected_players != expected_players {
        return Err(format!(
            "continued multiplayer session expects {} players, replacement requested {expected_players}",
            continuation.expected_players
        ));
    }
    Ok(slot.take())
}

/// Native reconnects within one process retain a transport identity across
/// outer-mission rebuilds. Browser peers prove their durable IndexedDB owner;
/// native peers use this process-held iroh key for the equivalent continuity.
fn native_client_secret_key() -> SecretKey {
    static KEY: OnceLock<SecretKey> = OnceLock::new();
    KEY.get_or_init(SecretKey::generate).clone()
}

// ─── Framing ─────────────────────────────────────────────────────

async fn write_frame(send: &mut SendStream, msg: &NetMsg) -> Result<(), String> {
    let bytes = encode_msg(msg);
    let class = net_frame_class(msg);
    if bytes.len() > class.absolute_limit() {
        return Err(format!(
            "outbound {class:?} frame of {} bytes exceeds {}-byte limit",
            bytes.len(),
            class.absolute_limit()
        ));
    }
    let len = u32::try_from(bytes.len()).map_err(|_| "outbound frame exceeds u32".to_string())?;
    let mut header = [0_u8; 5];
    header[0] = class as u8;
    header[1..].copy_from_slice(&len.to_le_bytes());
    send.write_all(&header)
        .await
        .map_err(|e| format!("write frame header: {e}"))?;
    send.write_all(&bytes)
        .await
        .map_err(|e| format!("write frame body: {e}"))?;
    Ok(())
}

/// Read one frame.  `Ok(None)` means the stream finished cleanly at a
/// frame boundary (graceful close).
async fn read_frame(
    recv: &mut RecvStream,
    policy: InboundFramePolicy,
) -> Result<Option<NetMsg>, String> {
    let mut header = [0u8; 5];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(format!("read frame header: {e}")),
    }
    let class = NetFrameClass::from_byte(header[0])?;
    let len = u32::from_le_bytes(header[1..].try_into().expect("four-byte frame length")) as usize;
    let limit = policy
        .limit(class)
        .ok_or_else(|| format!("{policy:?} may not send {class:?} frames"))?;
    if len > limit {
        return Err(format!(
            "inbound {class:?} frame of {len} bytes exceeds {limit}-byte {policy:?} limit"
        ));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| format!("read frame body: {e}"))?;
    let message = decode_msg(&buf).map_err(|e| format!("decode frame: {e}"))?;
    if net_frame_class(&message) != class {
        return Err(format!(
            "declared {class:?} frame decoded as {:?}",
            net_frame_class(&message)
        ));
    }
    Ok(Some(message))
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

// ─── Server ──────────────────────────────────────────────────────

/// Handle to a running multiplayer server.
///
/// Shutdown is deterministic: the shutdown signal makes the runtime
/// close the iroh endpoint (which ends the accept loop and every peer
/// connection), the outgoing pump stops, and the runtime thread plus
/// its bridge thread are joined before `shutdown` returns.
pub struct ServerHandle {
    /// `(local_seat, mission_seed)` the server is operating with.
    pub local_seat: PlayerId,
    pub mission_seed: u64,
    session_id: MultiplayerSessionId,
    endpoint_id: EndpointId,
    endpoint_addr: EndpointAddr,
    host_key: SecretKey,
    mission_id: String,
    context: Arc<ServerContext>,
    preserve_on_shutdown: bool,
    cancellation: Arc<AtomicBool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    runtime_thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn session_id(&self) -> MultiplayerSessionId {
        self.session_id
    }

    /// The stable public id peers dial to reach this server.
    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }

    /// Full endpoint address (id + transport addresses) as a connect
    /// string for [`connect_client`].  Lets peers dial explicit direct
    /// addresses without relay/DNS lookup (tests, LAN-only setups).
    pub fn connect_string(&self) -> String {
        serde_json::to_string(&self.endpoint_addr).expect("EndpointAddr serialization cannot fail")
    }

    pub fn browser_join_ticket(
        &self,
        content_edition: super::join_ticket::BrowserContentEdition,
        content_identity_sha256: String,
        mission_profile_id: Option<u32>,
        expected_players: u32,
    ) -> Result<super::join_ticket::BrowserJoinTicket, String> {
        super::join_ticket::BrowserJoinTicket::issue(
            &self.host_key,
            &self.endpoint_addr,
            self.session_id.0,
            current_epoch_ms() / 1000,
            content_edition,
            content_identity_sha256,
            self.mission_id.clone(),
            mission_profile_id,
            expected_players,
        )
    }

    pub fn shutdown(&mut self) {
        if self.preserve_on_shutdown {
            publish_context_continuation(&self.context);
            self.preserve_on_shutdown = false;
        }
        self.cancellation.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.runtime_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer server runtime panicked during shutdown");
        }
    }

    pub(super) fn preserve_session_for_next_mission(&mut self) {
        self.preserve_on_shutdown = true;
    }
}

fn publish_context_continuation(context: &ServerContext) {
    let peers = context.peers.lock();
    let mut owner_seats = peers.disconnected_seats.clone();
    owner_seats.extend(peers.owners.iter().map(|(&seat, &owner)| (owner, seat)));
    publish_host_session_continuation(HostSessionContinuation {
        host_endpoint_id: context.host_endpoint_id,
        session_id: context.session_id,
        expected_players: peers.expected_players,
        owner_seats,
        relay_url: context.relay_url.lock().clone(),
    });
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Per-peer state tracked by the server.  Wrapped in an `Arc<Mutex<>>`
/// so the accept task, the outgoing pump, and each per-peer task can
/// share access.
struct ServerPeers {
    /// Next [`PlayerId`] to assign for a peer with a nickname the
    /// server has not seen before.  Starts at 1 — seat 0 is the host.
    next_seat: u8,
    /// Active peers, keyed by their assigned [`PlayerId`].  The value
    /// is the sender used to push outbound frames into that peer's
    /// writer task.
    senders: HashMap<u8, UnboundedSender<NetMsg>>,
    /// Presentation names per active seat. Names never grant authority.
    nicknames: HashMap<u8, String>,
    owners: HashMap<u8, PeerOwner>,
    disconnected_seats: HashMap<PeerOwner, u8>,
    session_generations: HashMap<u8, u64>,
    next_session_generation: u64,
    expected_players: u32,
    host_ready_frame: Option<u32>,
    ready_seats: HashMap<u8, u32>,
    begin_sent: Option<(u32, u64)>,
    snapshot_transition: Option<PendingSnapshotTransition>,
}

struct PendingSnapshotTransition {
    id: robin_engine::multiplayer::SnapshotTransitionId,
    payload: robin_engine::multiplayer::SnapshotTransitionPayload,
    awaiting: HashSet<u8>,
}

impl ServerPeers {
    fn new(expected_players: u32) -> Self {
        Self {
            next_seat: 1,
            senders: HashMap::new(),
            nicknames: HashMap::new(),
            owners: HashMap::new(),
            disconnected_seats: HashMap::new(),
            session_generations: HashMap::new(),
            next_session_generation: 1,
            expected_players,
            host_ready_frame: None,
            ready_seats: HashMap::new(),
            begin_sent: None,
            snapshot_transition: None,
        }
    }

    fn from_continuation(continuation: &HostSessionContinuation) -> Self {
        let next_seat = continuation
            .owner_seats
            .values()
            .copied()
            .max()
            .map_or(1, |seat| {
                seat.checked_add(1).expect("multiplayer seat overflow")
            });
        Self {
            next_seat,
            senders: HashMap::new(),
            nicknames: HashMap::new(),
            owners: HashMap::new(),
            disconnected_seats: continuation.owner_seats.clone(),
            session_generations: HashMap::new(),
            next_session_generation: 1,
            expected_players: continuation.expected_players,
            host_ready_frame: None,
            ready_seats: HashMap::new(),
            begin_sent: None,
            snapshot_transition: None,
        }
    }

    fn owner_seat(&self, owner: PeerOwner) -> Option<u8> {
        self.owners
            .iter()
            .find_map(|(&seat, active_owner)| (*active_owner == owner).then_some(seat))
            .or_else(|| self.disconnected_seats.get(&owner).copied())
    }

    fn claim_seat(
        &mut self,
        owner: PeerOwner,
        nickname: &str,
        sender: UnboundedSender<NetMsg>,
    ) -> Result<(u8, u64), String> {
        let seat = if let Some(active) = self
            .owners
            .iter()
            .find_map(|(&seat, active_owner)| (*active_owner == owner).then_some(seat))
        {
            self.disconnected_seats.remove(&owner);
            active
        } else if let Some(disconnected) = self.disconnected_seats.remove(&owner) {
            disconnected
        } else {
            if self.next_seat as u32 >= self.expected_players {
                return Err(format!(
                    "multiplayer session already has its configured {} players",
                    self.expected_players
                ));
            }
            let next = self.next_seat;
            self.next_seat = next
                .checked_add(1)
                .ok_or_else(|| "multiplayer seat overflow".to_string())?;
            next
        };
        let generation = self.next_session_generation;
        self.next_session_generation = generation
            .checked_add(1)
            .ok_or_else(|| "multiplayer session generation overflow".to_string())?;
        self.senders.insert(seat, sender);
        self.nicknames.insert(seat, nickname.to_owned());
        self.owners.insert(seat, owner);
        self.session_generations.insert(seat, generation);
        self.ready_seats.remove(&seat);
        Ok((seat, generation))
    }

    fn release_seat_if_owner(&mut self, seat: u8, owner: PeerOwner, generation: u64) -> bool {
        if self.session_generations.get(&seat) != Some(&generation)
            || self.owners.get(&seat) != Some(&owner)
        {
            return false;
        }
        self.senders.remove(&seat);
        self.nicknames.remove(&seat).unwrap_or_else(|| {
            panic!("authenticated active multiplayer seat {seat} has no nickname")
        });
        self.owners
            .remove(&seat)
            .unwrap_or_else(|| panic!("authenticated active multiplayer seat {seat} has no owner"));
        self.session_generations.remove(&seat);
        self.ready_seats.remove(&seat);
        self.disconnected_seats.insert(owner, seat);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PeerOwner {
    Native([u8; 32]),
    Browser([u8; 32]),
}

fn take_committed_snapshot_transition(
    peers: &mut ServerPeers,
) -> Option<(
    robin_engine::multiplayer::SnapshotTransitionId,
    Vec<UnboundedSender<NetMsg>>,
)> {
    let transition = peers.snapshot_transition.as_ref()?;
    if !transition.awaiting.is_empty() {
        return None;
    }
    let id = peers
        .snapshot_transition
        .take()
        .expect("checked transition exists")
        .id;
    let senders = std::mem::take(&mut peers.senders).into_values().collect();
    Some((id, senders))
}

fn retain_transition_peer_for_reconnect(peers: &mut ServerPeers, seat: u8) {
    if let Some(transition) = peers.snapshot_transition.as_mut() {
        // A participant which loses its stream must validate again on the
        // replacement stream, even if its prior acknowledgement raced the
        // disconnect. Never shrink the barrier because of connectivity.
        transition.awaiting.insert(seat);
    }
}

fn commit_snapshot_transition(
    context: &ServerContext,
    committed: Option<(
        robin_engine::multiplayer::SnapshotTransitionId,
        Vec<UnboundedSender<NetMsg>>,
    )>,
) {
    let Some((id, senders)) = committed else {
        return;
    };
    publish_context_continuation(context);
    for sender in &senders {
        sender
            .send(NetMsg::CommitSnapshotTransition { id })
            .unwrap_or_else(|_| {
                panic!("snapshot transition commit queue closed before peer delivery")
            });
    }
    context
        .incoming_tx
        .send(NetEvent::CommitSnapshotTransition { id })
        .unwrap_or_else(|_| panic!("snapshot transition host event channel is closed"));
    // Dropping the last queue handles closes every old mission stream after
    // the commit frame has drained. All participants rebuild the transport
    // against the replacement mission and re-enter through its ready barrier.
    drop(senders);
}

fn maybe_begin_sim_locked(
    peers: &mut ServerPeers,
) -> Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)> {
    if peers.begin_sent.is_some() {
        return None;
    }
    let host_frame = peers.host_ready_frame?;
    let active_peer_count = peers.senders.len() as u32;
    let expected_peer_count = peers.expected_players.saturating_sub(1);
    if active_peer_count < expected_peer_count {
        return None;
    }
    if !peers
        .senders
        .keys()
        .all(|seat| peers.ready_seats.contains_key(seat))
    {
        return None;
    }

    let begin_frame = peers
        .ready_seats
        .values()
        .copied()
        .fold(host_frame, u32::max);
    let start_epoch_ms = current_epoch_ms().saturating_add(500);
    let senders = peers.senders.values().cloned().collect();
    peers.begin_sent = Some((begin_frame, start_epoch_ms));
    Some((begin_frame, start_epoch_ms, senders))
}

/// Per-session context shared by every server task.
struct ServerContext {
    peers: Mutex<ServerPeers>,
    incoming_tx: Sender<NetEvent>,
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    host_endpoint_id: EndpointId,
    session_id: MultiplayerSessionId,
    continued_session: bool,
    relay_url: Mutex<Option<iroh::RelayUrl>>,
    speech_timing_locale: Option<String>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    cancellation: Arc<AtomicBool>,
}

/// Start a multiplayer server on this install's persistent iroh
/// identity.  The server runs the host seat (seat 0) locally — the
/// returned [`NetEvent`] stream will receive each peer's inputs and
/// seat-join/leave events.  The local process should also push its
/// own [`PlayerCommand`]s into `outgoing_rx` via the sibling sender
/// so they are broadcast to peers and folded into the local input
/// batch.
///
/// Peers connect to [`ServerHandle::endpoint_id`], which equals
/// [`super::identity::local_endpoint_id_string`] — known before this
/// call, so matchmaking can advertise it ahead of mission launch.
#[allow(clippy::too_many_arguments)]
pub fn start_server(
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    speech_timing_locale: Option<String>,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    expected_players: u32,
    browser_join_enabled: bool,
) -> std::io::Result<ServerHandle> {
    let key = game_secret_key().map_err(std::io::Error::other)?;
    start_server_inner(
        key,
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        expected_players,
        browser_join_enabled,
    )
}

/// [`start_server`] with an explicit identity key.  Tests use this to
/// avoid touching the per-install on-disk identity.
#[allow(clippy::too_many_arguments)]
pub fn start_server_with_key(
    key: SecretKey,
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    speech_timing_locale: Option<String>,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    expected_players: u32,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        key,
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        expected_players,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_server_inner(
    key: SecretKey,
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    speech_timing_locale: Option<String>,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    expected_players: u32,
    browser_join_enabled: bool,
) -> std::io::Result<ServerHandle> {
    robin_engine::multiplayer::validate_display_name(&host_nickname)
        .map_err(std::io::Error::other)?;
    robin_engine::multiplayer::validate_mission_id(&mission_id).map_err(std::io::Error::other)?;
    if !(1..=super::join_ticket::MAX_MULTIPLAYER_PLAYERS).contains(&expected_players) {
        return Err(std::io::Error::other(format!(
            "multiplayer expected-player count must be between 1 and {}, got {expected_players}",
            super::join_ticket::MAX_MULTIPLAYER_PLAYERS
        )));
    }
    let host_endpoint_id = key.public();
    let continuation = take_host_session_continuation(host_endpoint_id, expected_players)
        .map_err(std::io::Error::other)?;
    let session_id = continuation.as_ref().map_or_else(
        || MultiplayerSessionId(SecretKey::generate().to_bytes()),
        |continuation| continuation.session_id,
    );
    let handle_key = key.clone();
    let handle_mission_id = mission_id.clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let (bridge_thread, outgoing_async_rx) = spawn_outgoing_bridge(
        "mp-server-outgoing-bridge",
        outgoing_rx,
        Arc::clone(&cancellation),
    )?;

    let context = Arc::new(ServerContext {
        peers: Mutex::new(continuation.as_ref().map_or_else(
            || ServerPeers::new(expected_players.max(1)),
            ServerPeers::from_continuation,
        )),
        incoming_tx,
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        host_endpoint_id,
        session_id,
        continued_session: continuation.is_some(),
        relay_url: Mutex::new(
            continuation
                .as_ref()
                .and_then(|continuation| continuation.relay_url.clone()),
        ),
        speech_timing_locale,
        frame_cursor,
        initial_snapshot,
        cancellation: Arc::clone(&cancellation),
    });

    let (startup_tx, startup_rx) =
        std::sync::mpsc::sync_channel::<Result<(EndpointId, EndpointAddr), String>>(1);
    let runtime_thread = thread::Builder::new().name("mp-server".into()).spawn({
        let context = Arc::clone(&context);
        move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = startup_tx.send(Err(format!("build tokio runtime: {e}")));
                    return;
                }
            };
            rt.block_on(run_server(
                key,
                context,
                outgoing_async_rx,
                startup_tx,
                shutdown_rx,
                browser_join_enabled,
            ));
            if bridge_thread.join().is_err() {
                tracing::error!("multiplayer server outgoing bridge panicked");
            }
        }
    })?;

    let (endpoint_id, endpoint_addr) = match startup_rx.recv() {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            return Err(std::io::Error::other(err));
        }
        Err(e) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            return Err(std::io::Error::other(format!(
                "server startup channel closed: {e}"
            )));
        }
    };
    tracing::info!(
        endpoint_id = %endpoint_id,
        seed = mission_seed,
        "multiplayer server listening on iroh"
    );

    Ok(ServerHandle {
        local_seat: PlayerId::HOST,
        mission_seed,
        session_id,
        endpoint_id,
        endpoint_addr,
        host_key: handle_key,
        mission_id: handle_mission_id,
        context,
        preserve_on_shutdown: false,
        cancellation,
        shutdown_tx,
        runtime_thread: Some(runtime_thread),
    })
}

async fn run_server(
    key: SecretKey,
    context: Arc<ServerContext>,
    outgoing_async_rx: UnboundedReceiver<NetOutbound>,
    startup_tx: std::sync::mpsc::SyncSender<Result<(EndpointId, EndpointAddr), String>>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    browser_join_enabled: bool,
) {
    let retained_relay = context.relay_url.lock().clone();
    let endpoint = match bind_endpoint_with_relay(key, GAME_ALPN, retained_relay).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            let _ = startup_tx.send(Err(e));
            return;
        }
    };
    if browser_join_enabled {
        if tokio::time::timeout(Duration::from_secs(15), endpoint.online())
            .await
            .is_err()
        {
            endpoint.close().await;
            let _ = startup_tx.send(Err(
                "iroh relay did not become reachable within 15 seconds; disable browser join-link publication for a native-only game"
                    .to_string(),
            ));
            return;
        }
        if endpoint.addr().relay_urls().next().is_none() {
            endpoint.close().await;
            let _ = startup_tx.send(Err(
                "iroh reported online without a relay URL; a browser invitation cannot be published"
                    .to_string(),
            ));
            return;
        }
    }
    *context.relay_url.lock() = endpoint.addr().relay_urls().next().cloned();
    let _ = startup_tx.send(Ok((endpoint.id(), endpoint.addr())));

    let pump = tokio::spawn(run_server_outgoing_pump(
        Arc::clone(&context),
        outgoing_async_rx,
    ));
    let accept = tokio::spawn(run_server_accept_loop(
        Arc::clone(&context),
        endpoint.clone(),
    ));

    // Root task: wait for shutdown, then close the endpoint (which
    // ends every peer connection and wakes the accept loop).
    let _ = shutdown_rx.wait_for(|stop| *stop).await;
    endpoint.close().await;
    accept.abort();
    pump.abort();
    let _ = accept.await;
    let _ = pump.await;
    tracing::info!("multiplayer server runtime stopped");
}

/// Take locally-produced messages from the game loop, stamp them with
/// seat 0 + a target frame, fan them out to every peer's writer
/// queue, AND echo them back into `incoming_tx` so the local game
/// loop applies them in the same input order every other machine
/// does.  Target frame = current sim frame + [`INPUT_DELAY_FRAMES`]
/// so peers (which receive the broadcast over the wire with some
/// latency) still have time to apply at the matching frame; if a peer
/// is already past the target, the rollback path picks up the slack.
async fn run_server_outgoing_pump(
    context: Arc<ServerContext>,
    mut outgoing_async_rx: UnboundedReceiver<NetOutbound>,
) {
    while let Some(msg) = outgoing_async_rx.recv().await {
        match msg {
            NetOutbound::Input {
                origin_frame,
                command,
            } => {
                let now = context.frame_cursor.load(Ordering::Relaxed);
                let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                let inp = PlayerInput::new(PlayerId::HOST, command);
                broadcast_input(&context, now, origin_frame, target, inp);
            }
            NetOutbound::StateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            } => {
                // Authoritative-host state hash: broadcast as a wire
                // `StateHash` to every peer.  No echo into our own
                // incoming channel — the local game loop already has
                // the value (it just computed the hash before pushing
                // here).
                broadcast_msg(
                    &context,
                    NetMsg::StateHash {
                        frame,
                        hash,
                        clock_frame,
                        ms_until_next_frame,
                    },
                );
            }
            NetOutbound::InitialSnapshot {
                frame,
                engine_bytes,
            } => {
                // A peer can complete the handshake before mission
                // setup has produced the frame-0 snapshot.  Push the
                // snapshot to all currently-connected peers as soon
                // as it exists; later peers still receive it through
                // the handshake cache.
                broadcast_msg(
                    &context,
                    NetMsg::InitialSnapshot {
                        frame,
                        engine_bytes,
                    },
                );
            }
            NetOutbound::ReadyToSim { frame } => {
                let begin = {
                    let mut p = context.peers.lock();
                    p.host_ready_frame = Some(frame);
                    maybe_begin_sim_locked(&mut p)
                };
                announce_begin_sim(&context, begin);
            }
            NetOutbound::ModalProposal { .. } => {
                tracing::error!("multiplayer host attempted to send a client-only modal proposal");
            }
            NetOutbound::ModalDecision {
                instance,
                kind,
                result,
                decision_frame,
            } => {
                if instance.session_id != context.session_id {
                    tracing::error!(
                        ?instance,
                        "multiplayer host rejected a modal decision for another session"
                    );
                    continue;
                }
                if let Err(error) = broadcast_msg_required(
                    &context,
                    NetMsg::ModalDecision {
                        instance,
                        kind,
                        result,
                        decision_frame,
                    },
                ) {
                    tracing::error!(%error, "authoritative modal broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::ReconnectForSnapshot { player_id, reason } => {
                assert_ne!(
                    player_id,
                    PlayerId::HOST,
                    "authoritative host cannot reconnect itself for a stale input"
                );
                let sender = context.peers.lock().senders.remove(&player_id.0);
                if let Some(sender) = sender {
                    tracing::warn!(
                        ?player_id,
                        %reason,
                        "multiplayer: dropping peer for full-snapshot resynchronization"
                    );
                    // Tell the peer why this otherwise-graceful stream close
                    // requires reconnecting. The queue drains this message
                    // before observing that its last sender was dropped.
                    let _ = sender.send(NetMsg::ReconnectRequired {
                        reason: reason.clone(),
                    });
                    drop(sender);
                } else {
                    tracing::warn!(
                        ?player_id,
                        %reason,
                        "multiplayer: stale-input peer was already disconnected"
                    );
                }
            }
            NetOutbound::ReconnectAllForSnapshot { reason } => {
                let senders = {
                    let mut peers = context.peers.lock();
                    peers.host_ready_frame = None;
                    peers.ready_seats.clear();
                    peers.begin_sent = None;
                    std::mem::take(&mut peers.senders)
                };
                tracing::warn!(
                    peers = senders.len(),
                    %reason,
                    "multiplayer: dropping every peer for full-snapshot resynchronization"
                );
                for sender in senders.values() {
                    let _ = sender.send(NetMsg::ReconnectRequired {
                        reason: reason.clone(),
                    });
                }
                drop(senders);
            }
            NetOutbound::BeginSnapshotTransition { id, payload } => {
                assert_eq!(
                    id.session_id, context.session_id,
                    "host snapshot transition belongs to another multiplayer session"
                );
                let committed = {
                    let mut peers = context.peers.lock();
                    assert!(
                        peers.snapshot_transition.is_none(),
                        "another multiplayer snapshot transition is already pending"
                    );
                    let awaiting = peers.senders.keys().copied().collect::<HashSet<_>>();
                    peers.snapshot_transition = Some(PendingSnapshotTransition {
                        id,
                        payload: payload.clone(),
                        awaiting,
                    });
                    let prepare = NetMsg::PrepareSnapshotTransition { id, payload };
                    // Keep the peer-state lock until every current writer has
                    // queued Prepare. Otherwise its reader could disconnect,
                    // empty the readiness set, and queue Commit first.
                    for sender in peers.senders.values() {
                        sender.send(prepare.clone()).unwrap_or_else(|_| {
                            panic!("snapshot transition prepare queue closed before peer delivery")
                        });
                    }
                    take_committed_snapshot_transition(&mut peers)
                };
                commit_snapshot_transition(&context, committed);
            }
            NetOutbound::SnapshotTransitionReady { .. } => {
                tracing::error!(
                    "multiplayer host attempted to acknowledge its own snapshot transition"
                );
            }
        }
    }
    tracing::info!("server outgoing pump stopped");
}

fn announce_begin_sim(
    context: &ServerContext,
    begin: Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>,
) {
    if let Some((begin_frame, start_epoch_ms, senders)) = begin {
        tracing::info!(
            frame = begin_frame,
            start_epoch_ms,
            "multiplayer: ready barrier complete"
        );
        let _ = context.incoming_tx.send(NetEvent::BeginSim {
            frame: begin_frame,
            start_epoch_ms,
        });
        for sender in senders {
            let _ = sender.send(NetMsg::BeginSim {
                frame: begin_frame,
                start_epoch_ms,
            });
        }
    }
}

/// Send one message to every connected peer's writer queue.
fn broadcast_msg(context: &ServerContext, msg: NetMsg) {
    let to_send: Vec<UnboundedSender<NetMsg>> = {
        let p = context.peers.lock();
        p.senders.values().cloned().collect()
    };
    for sender in to_send {
        let _ = sender.send(msg.clone());
    }
}

/// Queue an authoritative message for every currently connected peer. A
/// closed writer queue is a fatal session split, not a best-effort diagnostic.
fn broadcast_msg_required(context: &ServerContext, msg: NetMsg) -> Result<(), String> {
    let to_send: Vec<(u8, UnboundedSender<NetMsg>)> = {
        let peers = context.peers.lock();
        peers
            .senders
            .iter()
            .map(|(seat, sender)| (*seat, sender.clone()))
            .collect()
    };
    for (seat, sender) in to_send {
        sender.send(msg.clone()).map_err(|_| {
            format!("authoritative multiplayer send queue for seat {seat} is closed")
        })?;
    }
    Ok(())
}

/// Send a [`NetMsg::BroadcastInput`] to every peer plus echo it into
/// the local game-loop event stream.  A send failure just means that
/// peer's writer task ended (its reader emits `DisconnectSeat` on the
/// way out).
fn broadcast_input(
    context: &ServerContext,
    server_frame: u32,
    origin_frame: u32,
    target_frame: u32,
    inp: PlayerInput,
) {
    // Local fan-in: feed the input back into our own game loop.
    let _ = context.incoming_tx.send(NetEvent::Input {
        server_frame,
        origin_frame,
        target_frame,
        input: inp.clone(),
    });

    let to_send: Vec<(u8, UnboundedSender<NetMsg>)> = {
        let p = context.peers.lock();
        p.senders.iter().map(|(k, v)| (*k, v.clone())).collect()
    };
    for (seat, sender) in to_send {
        if sender
            .send(NetMsg::BroadcastInput {
                server_frame,
                origin_frame,
                target_frame,
                input: inp.clone(),
            })
            .is_err()
        {
            tracing::warn!(seat, "broadcast send to peer failed");
        }
    }
}

async fn run_server_accept_loop(context: Arc<ServerContext>, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let context = Arc::clone(&context);
        tokio::spawn(async move {
            let cancelled = context.cancellation.load(Ordering::Acquire);
            if let Err(e) = handle_incoming_peer(&context, incoming).await
                && !cancelled
                && !context.cancellation.load(Ordering::Acquire)
            {
                tracing::warn!("incoming peer handler ended: {e}");
            }
        });
    }
    tracing::info!("multiplayer accept loop stopped");
}

async fn handle_incoming_peer(
    context: &ServerContext,
    incoming: iroh::endpoint::Incoming,
) -> Result<(), String> {
    let conn = incoming
        .await
        .map_err(|e| format!("peer connecting: {e}"))?;
    let remote_id = conn.remote_id();
    let peer_id = remote_id.to_string();
    tracing::info!(peer = %peer_id, "incoming connection");

    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(|e| format!("accept peer stream: {e}"))?;

    // Receive Hello.  Reject anything else.
    let (nickname, browser_auth) =
        match read_frame(&mut recv, InboundFramePolicy::ClientHello).await? {
            Some(NetMsg::Hello {
                protocol_version,
                nickname,
                browser_auth,
            }) => {
                if protocol_version != NET_PROTOCOL_VERSION {
                    let reason = format!(
                        "protocol mismatch (peer={protocol_version}, server={NET_PROTOCOL_VERSION})"
                    );
                    reject_opening(&mut send, &reason).await;
                    return Err(reason);
                }
                (nickname, browser_auth)
            }
            Some(other) => {
                let reason = format!("expected Hello, got {other:?}");
                reject_opening(&mut send, &reason).await;
                return Err(reason);
            }
            None => return Err("connection closed before Hello".to_string()),
        };

    let owner = match authenticate_peer(context, remote_id, browser_auth.as_ref()) {
        Ok(owner) => owner,
        Err(reason) => {
            reject_opening(&mut send, &reason).await;
            return Err(reason);
        }
    };

    // Assign a seat — reuse the previously-held one if this nickname
    // is a returning peer.  Otherwise allocate the next fresh seat.
    let seat_claim = {
        let mut p = context.peers.lock();
        let returning_seat = p.owner_seat(owner);
        if let Some(transition) = p.snapshot_transition.as_ref()
            && !returning_seat.is_some_and(|seat| transition.awaiting.contains(&seat))
        {
            return Err(
                "host is changing missions; only pending participants may reconnect".to_string(),
            );
        }
        let (write_tx, write_rx) = unbounded_channel::<NetMsg>();
        p.claim_seat(owner, &nickname, write_tx)
            .map(|(seat, generation)| (seat, generation, write_rx))
    };
    let (assigned_seat_u8, session_generation, mut write_rx) = match seat_claim {
        Ok(claim) => claim,
        Err(reason) => {
            reject_opening(&mut send, &reason).await;
            return Err(reason);
        }
    };
    let assigned_seat = PlayerId(assigned_seat_u8);

    // Queue Welcome for this peer.  Goes through the writer queue so
    // the writer task is the only thing that touches the outbound
    // half of the stream.  If the host has cached an initial-state
    // snapshot we follow up with that — mid-mission joiners adopt it
    // instead of trying to reproduce engine init from seed alone.
    {
        let p = context.peers.lock();
        if let Some(sender) = p.senders.get(&assigned_seat_u8) {
            sender
                .send(NetMsg::Welcome {
                    your_seat: assigned_seat,
                    session_id: context.session_id,
                    mission_id: context.mission_id.clone(),
                    mission_seed: context.mission_seed,
                    sim_config: context.sim_config,
                    speech_timing_locale: context.speech_timing_locale.clone(),
                    host_nickname: context.host_nickname.clone(),
                })
                .map_err(|_| "writer queue closed before Welcome")?;
            // `InitialSnapshot` is a plain std mutex shared with the
            // game loop; the snapshot value is only ever replaced
            // wholesale, so recover it if a prior holder panicked
            // instead of silently skipping the snapshot send.
            let snapshot_frame = if let Some((frame, bytes)) = context
                .initial_snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                tracing::info!(
                    seat = assigned_seat_u8,
                    frame,
                    bytes = bytes.len(),
                    "sending initial snapshot to peer"
                );
                let _ = sender.send(NetMsg::InitialSnapshot {
                    frame,
                    engine_bytes: bytes,
                });
                Some(frame)
            } else {
                None
            };
            if let Some((frame, start_epoch_ms)) = p.begin_sent {
                let begin_frame =
                    snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
                let begin_start_epoch_ms = if begin_frame != frame {
                    current_epoch_ms().saturating_add(100)
                } else {
                    start_epoch_ms
                };
                let _ = sender.send(NetMsg::BeginSim {
                    frame: begin_frame,
                    start_epoch_ms: begin_start_epoch_ms,
                });
            }
            if let Some(transition) = p.snapshot_transition.as_ref() {
                sender
                    .send(NetMsg::PrepareSnapshotTransition {
                        id: transition.id,
                        payload: transition.payload.clone(),
                    })
                    .map_err(|_| "writer queue closed before transition Prepare")?;
            }
        }
    }

    // Broadcast ConnectSeat as a regular tagged BroadcastInput.
    // Routing through `broadcast_input` stamps `target_frame` from
    // the shared cursor (so the local echo and every peer apply the
    // ConnectSeat at the same simulation frame), keeping the seat's
    // arrival deterministic across machines.  Receivers fold it into
    // the engine's `seats` vec just like any other input.
    {
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::ConnectSeat {
                player_id: assigned_seat,
                nickname: nickname.clone(),
            },
        );
        broadcast_input(context, now, now, target, inp);
    }

    // Writer half: drain the peer's queue onto the stream.  Reader
    // half: every Input received gets stamped with the peer's
    // assigned seat (defensive — the client tags its own outgoing
    // too, but we don't trust the wire) and a target frame derived
    // from the server's current sim frame at receive time, before
    // broadcasting.  Both halves run in this task via `select!` so
    // either side ending tears the peer down.
    let result = {
        let writer = async {
            while let Some(msg) = write_rx.recv().await {
                write_frame(&mut send, &msg).await?;
            }
            // Queue closed: seat was dropped (shutdown or cleanup).
            Ok::<(), String>(())
        };
        let reader = run_server_peer_reader(context, assigned_seat, &mut recv);
        tokio::select! {
            result = reader => result,
            result = writer => result.map_err(|e| format!("peer writer: {e}")),
        }
    };

    // On disconnect: drop the peer slot, broadcast DisconnectSeat.
    // The nickname is parked in `disconnected_seats` so a future
    // `Hello` from the same nickname is reassigned the same seat —
    // the sim preserves the seat's selection / hotgroups across the
    // disconnect, so the rejoining peer takes back ownership of the
    // PCs they were controlling.
    let released = {
        let mut p = context.peers.lock();
        let released = p.release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        if released {
            retain_transition_peer_for_reconnect(&mut p, assigned_seat_u8);
        }
        released
    };
    if released && !context.cancellation.load(Ordering::Acquire) {
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::DisconnectSeat {
                player_id: assigned_seat,
            },
        );
        broadcast_input(context, now, now, target, inp);
    }
    conn.close(CLOSE_GRACEFUL.into(), b"session over");

    result
}

async fn reject_opening(send: &mut SendStream, reason: &str) {
    let reason = reason
        .chars()
        .take(robin_engine::multiplayer::MAX_REJECT_REASON_BYTES)
        .collect::<String>();
    if let Err(error) = write_frame(send, &NetMsg::Reject { reason }).await {
        tracing::debug!(%error, "failed to send multiplayer opening rejection");
    }
}

fn authenticate_peer(
    context: &ServerContext,
    remote_id: EndpointId,
    browser_auth: Option<&BrowserPeerAuth>,
) -> Result<PeerOwner, String> {
    let Some(auth) = browser_auth else {
        return Ok(PeerOwner::Native(*remote_id.as_bytes()));
    };
    let ticket = super::join_ticket::BrowserJoinTicket::decode_authenticated(&auth.join_code)?;
    let payload = ticket.payload();
    if payload.host_endpoint_id != context.host_endpoint_id.to_string()
        || ticket.session_id()? != context.session_id.0
        || payload.expected_players != context.peers.lock().expected_players
    {
        return Err(
            "browser invitation does not belong to this exact hosted mission session".to_string(),
        );
    }
    if payload.mission_id != context.mission_id && !context.continued_session {
        return Err("browser invitation belongs to another hosted mission".to_string());
    }
    let owner = PeerOwner::Browser(auth.durable_public_key);
    let use_kind = if context.peers.lock().owner_seat(owner).is_some() {
        super::join_ticket::InvitationUse::RedeemedReconnect
    } else {
        super::join_ticket::InvitationUse::Initial
    };
    ticket.validate_use_at(current_epoch_ms() / 1000, use_kind)?;
    let public_key = iroh::PublicKey::from_bytes(&auth.durable_public_key)
        .map_err(|error| format!("invalid durable browser public key: {error}"))?;
    let signature_bytes: [u8; iroh::Signature::LENGTH] = auth
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| "browser seat proof signature must be 64 bytes".to_string())?;
    let signature = iroh::Signature::from_bytes(&signature_bytes);
    let message = browser_seat_proof_message(
        context.session_id.0,
        *context.host_endpoint_id.as_bytes(),
        *remote_id.as_bytes(),
    );
    public_key
        .verify(&message, &signature)
        .map_err(|_| "browser seat proof does not bind this session and transport".to_string())?;
    Ok(owner)
}

async fn run_server_peer_reader(
    context: &ServerContext,
    seat: PlayerId,
    recv: &mut RecvStream,
) -> Result<(), String> {
    loop {
        match read_frame(recv, InboundFramePolicy::ClientToServer).await? {
            Some(NetMsg::Input {
                origin_frame,
                command,
            }) => {
                let now = context.frame_cursor.load(Ordering::Relaxed);
                let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                let inp = PlayerInput::new(seat, command);
                broadcast_input(context, now, origin_frame, target, inp);
            }
            Some(NetMsg::Note(s)) => {
                tracing::info!(?seat, note = %s, "peer note");
            }
            Some(NetMsg::ModalProposal {
                instance,
                kind,
                result,
                requested_frame,
            }) => {
                if instance.session_id != context.session_id {
                    return Err(format!(
                        "peer {seat:?} submitted a modal proposal for another session"
                    ));
                }
                context
                    .incoming_tx
                    .send(NetEvent::ModalProposal {
                        from: seat,
                        instance,
                        kind,
                        result,
                        requested_frame,
                    })
                    .map_err(|_| "host modal proposal channel is closed".to_string())?;
            }
            Some(NetMsg::ModalDecision { .. }) => {
                return Err(format!(
                    "peer {seat:?} attempted an authoritative modal decision"
                ));
            }
            Some(NetMsg::ReadyToSim { frame }) => {
                let begin = {
                    let mut p = context.peers.lock();
                    p.ready_seats.insert(seat.0, frame);
                    maybe_begin_sim_locked(&mut p)
                };
                announce_begin_sim(context, begin);
            }
            Some(NetMsg::SnapshotTransitionReady { id }) => {
                if id.session_id != context.session_id {
                    return Err(format!(
                        "peer {seat:?} acknowledged a snapshot transition for another session"
                    ));
                }
                let committed = {
                    let mut peers = context.peers.lock();
                    let transition = peers.snapshot_transition.as_mut().ok_or_else(|| {
                        format!("peer {seat:?} acknowledged no active snapshot transition")
                    })?;
                    if transition.id != id {
                        return Err(format!(
                            "peer {seat:?} acknowledged snapshot transition {id:?}, active is {:?}",
                            transition.id
                        ));
                    }
                    if !transition.awaiting.remove(&seat.0) {
                        return Err(format!(
                            "peer {seat:?} duplicated or was not expected for snapshot transition {id:?}"
                        ));
                    }
                    take_committed_snapshot_transition(&mut peers)
                };
                commit_snapshot_transition(context, committed);
            }
            Some(
                NetMsg::PrepareSnapshotTransition { .. } | NetMsg::CommitSnapshotTransition { .. },
            ) => {
                return Err(format!(
                    "peer {seat:?} attempted a host-only snapshot transition message"
                ));
            }
            Some(other) => {
                tracing::debug!(?seat, ?other, "ignoring inbound message from peer");
            }
            None => return Ok(()),
        }
    }
}

// ─── Client ──────────────────────────────────────────────────────

/// Handle to an active client connection.
pub struct ClientHandle {
    /// Seat assigned by the server.  `None` until the handshake
    /// completes.  Game loop reads this to set `host.local_seat`.
    pub assigned_seat: Arc<Mutex<Option<PlayerId>>>,
    session_id: MultiplayerSessionId,
    /// Mission RNG seed announced by the server in `Welcome`.  The
    /// client adopts this seed for its engine init so the local sim
    /// rolls match the host's.
    pub mission_seed: Option<u64>,
    pub mission_sim_config: Option<robin_engine::engine::SimConfig>,
    pub speech_timing_locale: Option<String>,
    pub mission_id: Option<String>,
    cancellation: Arc<AtomicBool>,
    io_thread: Option<JoinHandle<()>>,
}

impl ClientHandle {
    pub fn session_id(&self) -> Option<MultiplayerSessionId> {
        Some(self.session_id)
    }

    pub fn assigned_seat(&self) -> Option<PlayerId> {
        *self.assigned_seat.lock()
    }

    pub fn mission_seed(&self) -> Option<u64> {
        self.mission_seed
    }

    pub fn mission_sim_config(&self) -> Option<robin_engine::engine::SimConfig> {
        self.mission_sim_config
    }

    pub fn speech_timing_locale(&self) -> Option<String> {
        self.speech_timing_locale.clone()
    }

    pub fn mission_id(&self) -> Option<&str> {
        self.mission_id.as_deref()
    }

    pub fn shutdown(&mut self) {
        self.cancellation.store(true, Ordering::Release);
        if let Some(handle) = self.io_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer client worker panicked during shutdown");
        }
    }
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Connect to a multiplayer server and run the I/O thread.  `addr` is
/// the host's endpoint id (or a full endpoint-address connect string,
/// see [`parse_connect_addr`]).  Returns once the handshake
/// completes; the assigned seat is reported through `incoming_tx` as
/// a [`NetEvent::AssignedLocalSeat`].
///
/// The client binds an *ephemeral* iroh identity: unlike hosting,
/// joining needs no stable id (rejoin seat reuse is keyed by
/// nickname), and an ephemeral key lets two clients share a machine
/// with the hosting install.
pub fn connect_client(
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    robin_engine::multiplayer::validate_display_name(&nickname).map_err(std::io::Error::other)?;
    let server_addr = parse_connect_addr(addr.as_ref()).map_err(std::io::Error::other)?;
    let addr_display = addr.as_ref().to_string();
    let assigned_seat = Arc::new(Mutex::new(None));
    let assigned_clone = Arc::clone(&assigned_seat);
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancellation_for_thread = Arc::clone(&cancellation);
    let (handshake_tx, handshake_rx) = std::sync::mpsc::sync_channel(1);
    let (bridge_thread, mut outgoing_async_rx) = spawn_outgoing_bridge(
        "mp-client-outgoing-bridge",
        outgoing_rx,
        Arc::clone(&cancellation),
    )?;
    let io_thread = thread::Builder::new()
        .name("mp-client".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = handshake_tx.send(Err(format!("build tokio runtime: {e}")));
                    return;
                }
            };
            let cancellation_for_io = Arc::clone(&cancellation_for_thread);
            rt.block_on(async move {
                run_client_io_async(
                    server_addr,
                    nickname,
                    incoming_tx,
                    &mut outgoing_async_rx,
                    assigned_clone,
                    handshake_tx,
                    cancellation_for_io,
                )
                .await;
            });
            cancellation_for_thread.store(true, Ordering::Release);
            if bridge_thread.join().is_err() {
                tracing::error!("multiplayer client outgoing bridge panicked");
            }
        })?;

    let (your_seat, session_id, mission_id, mission_seed, sim_config, speech_timing_locale) =
        match handshake_rx.recv() {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                cancellation.store(true, Ordering::Release);
                let _ = io_thread.join();
                return Err(std::io::Error::other(format!("initial handshake: {err}")));
            }
            Err(e) => {
                cancellation.store(true, Ordering::Release);
                let _ = io_thread.join();
                return Err(std::io::Error::other(format!(
                    "initial handshake channel closed: {e}"
                )));
            }
        };
    tracing::info!(
        addr = %addr_display,
        ?your_seat,
        seed = mission_seed,
        "multiplayer client connected"
    );

    Ok(ClientHandle {
        assigned_seat,
        session_id,
        mission_seed: Some(mission_seed),
        mission_sim_config: Some(sim_config),
        speech_timing_locale,
        mission_id: Some(mission_id),
        cancellation,
        io_thread: Some(io_thread),
    })
}

/// A live client session: the connection plus its single
/// bidirectional message stream.
struct ClientSession {
    // Held so the QUIC connection stays open for the streams' lifetime.
    _conn: Connection,
    send: SendStream,
    recv: RecvStream,
}

type Handshake = (
    ClientSession,
    PlayerId,
    MultiplayerSessionId,
    String,
    u64,
    robin_engine::engine::SimConfig,
    Option<String>,
);

/// One round of (connect → open stream → Hello → Welcome).  Used both
/// for the initial handshake and for the auto-retry path after
/// disconnects.
async fn handshake_async(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
) -> Result<Handshake, String> {
    let conn = endpoint
        .connect(server_addr.clone(), GAME_ALPN)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("open stream: {e}"))?;

    write_frame(
        &mut send,
        &NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: nickname.to_string(),
            browser_auth: None,
        },
    )
    .await
    .map_err(|e| format!("send Hello: {e}"))?;

    match read_frame(&mut recv, InboundFramePolicy::ServerToClient).await? {
        Some(NetMsg::Welcome {
            your_seat,
            session_id,
            mission_id,
            mission_seed,
            sim_config,
            speech_timing_locale,
            host_nickname,
        }) => {
            tracing::info!(
                ?your_seat,
                seed = mission_seed,
                host = %host_nickname,
                "welcomed by server"
            );
            Ok((
                ClientSession {
                    _conn: conn,
                    send,
                    recv,
                },
                your_seat,
                session_id,
                mission_id,
                mission_seed,
                sim_config,
                speech_timing_locale,
            ))
        }
        Some(NetMsg::Reject { reason }) => Err(format!("host rejected connection: {reason}")),
        Some(other) => Err(format!("expected Welcome, got {other:?}")),
        None => Err("connection closed before Welcome".to_string()),
    }
}

async fn handshake_or_cancel(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
    cancellation: &AtomicBool,
) -> Option<Result<Handshake, String>> {
    tokio::select! {
        result = handshake_async(endpoint, server_addr, nickname) => Some(result),
        _ = wait_for_cancel(cancellation) => None,
    }
}

fn validate_reconnect_state(
    expected_seat: PlayerId,
    expected_mission_id: &str,
    expected_seed: u64,
    expected_config: robin_engine::engine::SimConfig,
    expected_speech_timing_locale: Option<&str>,
    expected_session_id: MultiplayerSessionId,
    seat: PlayerId,
    mission_id: &str,
    seed: u64,
    config: robin_engine::engine::SimConfig,
    speech_timing_locale: Option<&str>,
    session_id: MultiplayerSessionId,
) -> Result<(), String> {
    if seat != expected_seat
        || mission_id != expected_mission_id
        || seed != expected_seed
        || config != expected_config
        || speech_timing_locale != expected_speech_timing_locale
        || session_id != expected_session_id
    {
        return Err(format!(
            "reconnect joined incompatible seat {seat:?} mission `{mission_id}` seed {seed} config {config:?} speech timing {speech_timing_locale:?} session {session_id:?}; expected seat {expected_seat:?} mission `{expected_mission_id}` seed {expected_seed} config {expected_config:?} speech timing {expected_speech_timing_locale:?} session {expected_session_id:?}"
        ));
    }
    Ok(())
}

fn validate_reconnect_session_id(
    expected: MultiplayerSessionId,
    actual: MultiplayerSessionId,
) -> Result<(), String> {
    if actual != expected {
        return Err(format!(
            "reconnect joined a different multiplayer session {actual:?}; expected {expected:?}"
        ));
    }
    Ok(())
}

/// Drive one connection until it ends, then auto-reconnect with
/// exponential backoff.  Returns when the game loop drops the
/// outgoing queue (`host.net` dropped) or shutdown is requested.
async fn run_client_io_async(
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<
        Result<
            (
                PlayerId,
                MultiplayerSessionId,
                String,
                u64,
                robin_engine::engine::SimConfig,
                Option<String>,
            ),
            String,
        >,
    >,
    cancellation: Arc<AtomicBool>,
) {
    let endpoint = match bind_endpoint(native_client_secret_key(), GAME_ALPN).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            let _ = initial_handshake_tx.send(Err(e));
            return;
        }
    };

    run_client_io_inner(
        &endpoint,
        server_addr,
        nickname,
        incoming_tx,
        outgoing_async_rx,
        assigned,
        initial_handshake_tx,
        cancellation,
    )
    .await;

    endpoint.close().await;
}

#[allow(clippy::too_many_arguments)]
async fn run_client_io_inner(
    endpoint: &Endpoint,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<
        Result<
            (
                PlayerId,
                MultiplayerSessionId,
                String,
                u64,
                robin_engine::engine::SimConfig,
                Option<String>,
            ),
            String,
        >,
    >,
    cancellation: Arc<AtomicBool>,
) {
    let (
        mut session,
        your_seat,
        session_id,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
    ) = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut backoff = std::time::Duration::from_millis(50);
        loop {
            if cancellation.load(Ordering::Acquire) {
                let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                return;
            }
            let Some(handshake) =
                handshake_or_cancel(endpoint, &server_addr, &nickname, &cancellation).await
            else {
                let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                return;
            };
            match handshake {
                Ok(result) => break result,
                Err(err) if tokio::time::Instant::now() < deadline => {
                    tracing::debug!("initial multiplayer handshake failed: {err}; retrying");
                    if sleep_or_cancel(backoff, &cancellation).await {
                        let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                        return;
                    }
                    backoff = (backoff * 2).min(std::time::Duration::from_millis(500));
                }
                Err(err) => {
                    let _ = initial_handshake_tx.send(Err(err));
                    return;
                }
            }
        }
    };

    *assigned.lock() = Some(your_seat);
    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(your_seat));
    let _ = incoming_tx.send(NetEvent::MissionConfig {
        mission_id: mission_id.clone(),
        rng_seed: mission_seed,
        sim_config,
        speech_timing_locale: speech_timing_locale.clone(),
    });
    let _ = initial_handshake_tx.send(Ok((
        your_seat,
        session_id,
        mission_id.clone(),
        mission_seed,
        sim_config,
        speech_timing_locale.clone(),
    )));

    let mut backoff = std::time::Duration::from_millis(500);
    loop {
        match run_session_async(session, &incoming_tx, outgoing_async_rx, &cancellation).await {
            SessionEnd::Graceful => break,
            SessionEnd::Drop(reason) => {
                let discarded = discard_session_outbound(outgoing_async_rx);
                tracing::warn!("client session ended: {reason}; reconnecting...");
                if discarded != 0 {
                    tracing::warn!(
                        discarded,
                        "multiplayer: discarded outbound commands from abandoned prediction session"
                    );
                }
                let _ = incoming_tx.send(NetEvent::Note(format!(
                    "disconnected: {reason}; reconnecting..."
                )));
                let _ = incoming_tx.send(NetEvent::Disconnected);
            }
            SessionEnd::OutgoingClosed => return,
        }

        if sleep_or_cancel(backoff, &cancellation).await {
            return;
        }
        backoff = (backoff * 2).min(std::time::Duration::from_secs(10));

        session = loop {
            if cancellation.load(Ordering::Acquire) {
                return;
            }
            let Some(handshake) =
                handshake_or_cancel(endpoint, &server_addr, &nickname, &cancellation).await
            else {
                return;
            };
            match handshake {
                Ok((
                    new_session,
                    new_seat,
                    new_session_id,
                    new_mission_id,
                    new_seed,
                    new_config,
                    new_speech_timing_locale,
                )) => {
                    if let Err(message) = validate_reconnect_session_id(session_id, new_session_id)
                    {
                        let _ = incoming_tx.send(NetEvent::Fatal(message));
                        return;
                    }
                    if let Err(message) = validate_reconnect_state(
                        your_seat,
                        &mission_id,
                        mission_seed,
                        sim_config,
                        speech_timing_locale.as_deref(),
                        session_id,
                        new_seat,
                        &new_mission_id,
                        new_seed,
                        new_config,
                        new_speech_timing_locale.as_deref(),
                        new_session_id,
                    ) {
                        let _ = incoming_tx.send(NetEvent::Fatal(message));
                        return;
                    }
                    tracing::info!(?new_seat, seed = new_seed, "client reconnected");
                    *assigned.lock() = Some(new_seat);
                    let _ = incoming_tx.send(NetEvent::Reconnected);
                    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(new_seat));
                    let _ = incoming_tx.send(NetEvent::MissionConfig {
                        mission_id: new_mission_id,
                        rng_seed: new_seed,
                        sim_config: new_config,
                        speech_timing_locale: new_speech_timing_locale,
                    });
                    let discarded = discard_session_outbound(outgoing_async_rx);
                    if discarded != 0 {
                        tracing::warn!(
                            discarded,
                            "multiplayer: discarded commands queued while transport was reconnecting"
                        );
                    }
                    backoff = std::time::Duration::from_millis(500);
                    break new_session;
                }
                Err(e) => {
                    tracing::warn!("reconnect failed: {e}; will retry in {backoff:?}");
                    if sleep_or_cancel(backoff, &cancellation).await {
                        return;
                    }
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(10));
                }
            }
        };
    }

    let _ = incoming_tx.send(NetEvent::Disconnected);
}

/// Why a client session ended.
enum SessionEnd {
    /// Server closed the stream cleanly.
    Graceful,
    /// Network error / unexpected drop — caller should retry.
    Drop(String),
    /// The game loop dropped the outgoing channel — caller should
    /// stop the I/O thread entirely (no retry).
    OutgoingClosed,
}

/// Throw away commands queued for a transport session whose prediction
/// future has been abandoned. Replaying them after the next handshake would
/// apply pre-disconnect input on top of the authoritative replacement
/// snapshot.
fn discard_session_outbound(outgoing_rx: &mut UnboundedReceiver<NetOutbound>) -> usize {
    let mut discarded = 0;
    while outgoing_rx.try_recv().is_ok() {
        discarded += 1;
    }
    discarded
}

/// Run one client session by racing a whole-session reader loop
/// against a whole-session writer loop, so local inputs are sent as
/// soon as the game loop queues them.  The reader and writer each own
/// their stream half for the session's lifetime — a `select!` over
/// individual `read_frame` calls would drop partially-read frames
/// when another branch fires first.
async fn run_session_async(
    session: ClientSession,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut UnboundedReceiver<NetOutbound>,
    cancellation: &AtomicBool,
) -> SessionEnd {
    let ClientSession {
        _conn,
        mut send,
        mut recv,
    } = session;
    let reader = async {
        loop {
            match read_frame(&mut recv, InboundFramePolicy::ServerToClient).await {
                Ok(Some(msg)) => {
                    if let Err(error) = handle_client_wire_msg(incoming_tx, msg) {
                        if !error.starts_with("host requires a full-snapshot reconnect:") {
                            let _ = incoming_tx.send(NetEvent::Fatal(error.clone()));
                        }
                        return SessionEnd::Drop(error);
                    }
                }
                Ok(None) => return SessionEnd::Graceful,
                Err(e) => return SessionEnd::Drop(e),
            }
        }
    };
    let writer = async {
        loop {
            let Some(outgoing) = outgoing_rx.recv().await else {
                return SessionEnd::OutgoingClosed;
            };
            if let Err(e) = send_client_outgoing(&mut send, outgoing).await {
                return SessionEnd::Drop(e);
            }
        }
    };
    tokio::select! {
        _ = wait_for_cancel(cancellation) => SessionEnd::OutgoingClosed,
        end = reader => end,
        end = writer => end,
    }
}

async fn wait_for_cancel(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
}

/// Sleep for a reconnect backoff, returning early when shutdown begins.
async fn sleep_or_cancel(duration: Duration, cancellation: &AtomicBool) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = wait_for_cancel(cancellation) => true,
    }
}

fn handle_client_wire_msg(incoming_tx: &Sender<NetEvent>, msg: NetMsg) -> Result<(), String> {
    match msg {
        NetMsg::BroadcastInput {
            server_frame,
            origin_frame,
            target_frame,
            input,
        } => {
            incoming_tx
                .send(NetEvent::Input {
                    server_frame,
                    origin_frame,
                    target_frame,
                    input,
                })
                .map_err(|_| "client network event channel is closed".to_string())?;
        }
        NetMsg::Note(s) => {
            let _ = incoming_tx.send(NetEvent::Note(s));
        }
        NetMsg::StateHash {
            frame,
            hash,
            clock_frame,
            ms_until_next_frame,
        } => {
            let _ = incoming_tx.send(NetEvent::PeerStateHash {
                frame,
                hash,
                clock_frame,
                ms_until_next_frame,
            });
        }
        NetMsg::InitialSnapshot {
            frame,
            engine_bytes,
        } => {
            let _ = incoming_tx.send(NetEvent::InitialSnapshot {
                frame,
                engine_bytes,
            });
        }
        NetMsg::BeginSim {
            frame,
            start_epoch_ms,
        } => {
            let _ = incoming_tx.send(NetEvent::BeginSim {
                frame,
                start_epoch_ms,
            });
        }
        NetMsg::ModalDecision {
            instance,
            kind,
            result,
            decision_frame,
        } => {
            incoming_tx
                .send(NetEvent::ModalDecision {
                    instance,
                    kind,
                    result,
                    decision_frame,
                })
                .map_err(|_| "client modal decision channel is closed".to_string())?;
        }
        NetMsg::ModalProposal { .. } => {
            return Err("server sent a client-only modal proposal".to_string());
        }
        NetMsg::ReconnectRequired { reason } => {
            return Err(format!("host requires a full-snapshot reconnect: {reason}"));
        }
        NetMsg::PrepareSnapshotTransition { id, payload } => {
            incoming_tx
                .send(NetEvent::PrepareSnapshotTransition { id, payload })
                .map_err(|_| "client snapshot transition channel is closed".to_string())?;
        }
        NetMsg::CommitSnapshotTransition { id } => {
            incoming_tx
                .send(NetEvent::CommitSnapshotTransition { id })
                .map_err(|_| "client snapshot transition channel is closed".to_string())?;
        }
        NetMsg::SnapshotTransitionReady { .. } => {
            return Err("server sent a client-only snapshot transition acknowledgement".into());
        }
        other => {
            tracing::debug!(?other, "ignoring unexpected wire message");
        }
    }
    Ok(())
}

async fn send_client_outgoing(send: &mut SendStream, outgoing: NetOutbound) -> Result<(), String> {
    match outgoing {
        NetOutbound::Input {
            origin_frame,
            command,
        } => {
            write_frame(
                send,
                &NetMsg::Input {
                    origin_frame,
                    command,
                },
            )
            .await?;
        }
        NetOutbound::StateHash { .. } => {
            // Clients don't broadcast hashes.
        }
        NetOutbound::InitialSnapshot { .. } => {
            // Clients do not publish authoritative snapshots.
        }
        NetOutbound::ReadyToSim { frame } => {
            write_frame(send, &NetMsg::ReadyToSim { frame }).await?;
        }
        NetOutbound::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        } => {
            write_frame(
                send,
                &NetMsg::ModalProposal {
                    instance,
                    kind,
                    result,
                    requested_frame,
                },
            )
            .await?;
        }
        NetOutbound::ModalDecision { .. } => {
            return Err("client attempted an authoritative modal decision".to_string());
        }
        NetOutbound::ReconnectForSnapshot { reason, .. } => {
            return Err(format!(
                "full-snapshot resynchronization requested: {reason}"
            ));
        }
        NetOutbound::ReconnectAllForSnapshot { reason } => {
            return Err(format!(
                "full-snapshot resynchronization requested: {reason}"
            ));
        }
        NetOutbound::BeginSnapshotTransition { .. } => {
            return Err("client attempted an authoritative snapshot transition".to_string());
        }
        NetOutbound::SnapshotTransitionReady { id } => {
            write_frame(send, &NetMsg::SnapshotTransitionReady { id }).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        HostSessionContinuation, PeerOwner, PendingSnapshotTransition, ServerPeers,
        discard_session_outbound, handle_client_wire_msg, retain_transition_peer_for_reconnect,
        take_committed_snapshot_transition, validate_reconnect_session_id,
        validate_reconnect_state,
    };
    use robin_engine::player_command::PlayerId;
    use std::collections::HashSet;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn replacement_transport_seeds_exact_authenticated_seats() {
        let browser = PeerOwner::Browser([7; 32]);
        let native = PeerOwner::Native([8; 32]);
        let continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 3,
            owner_seats: std::collections::HashMap::from([(browser, 1), (native, 2)]),
            relay_url: None,
        };
        let mut peers = ServerPeers::from_continuation(&continuation);

        let (browser_tx, _browser_rx) = unbounded_channel();
        let (browser_seat, _) = peers
            .claim_seat(browser, "renamed browser", browser_tx)
            .unwrap();
        let (native_tx, _native_rx) = unbounded_channel();
        let (native_seat, _) = peers
            .claim_seat(native, "renamed native", native_tx)
            .unwrap();

        assert_eq!(browser_seat, 1);
        assert_eq!(native_seat, 2);
        let (intruder_tx, _intruder_rx) = unbounded_channel();
        assert!(
            peers
                .claim_seat(PeerOwner::Browser([9; 32]), "same nickname", intruder_tx)
                .is_err(),
            "a replacement session may not allocate beyond its retained roster"
        );
    }

    #[test]
    fn snapshot_transition_waits_for_every_current_peer() {
        let session_id = robin_engine::multiplayer::MultiplayerSessionId([4; 32]);
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id,
            sequence: 2,
        };
        let mut peers = ServerPeers::new(0);
        for seat in [1, 2] {
            let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
            peers.senders.insert(seat, sender);
        }
        peers.snapshot_transition = Some(PendingSnapshotTransition {
            id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 7,
                save_bytes: vec![1, 2, 3],
            },
            awaiting: HashSet::from([1, 2]),
        });

        assert!(take_committed_snapshot_transition(&mut peers).is_none());
        peers
            .snapshot_transition
            .as_mut()
            .unwrap()
            .awaiting
            .remove(&1);
        assert!(take_committed_snapshot_transition(&mut peers).is_none());
        peers
            .snapshot_transition
            .as_mut()
            .unwrap()
            .awaiting
            .remove(&2);
        let (committed_id, senders) =
            take_committed_snapshot_transition(&mut peers).expect("all peers acknowledged");
        assert_eq!(committed_id, id);
        assert_eq!(senders.len(), 2);
        assert!(peers.senders.is_empty());
        assert!(peers.snapshot_transition.is_none());
    }

    #[test]
    fn disconnected_transition_peer_must_ack_again_after_reconnect() {
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: robin_engine::multiplayer::MultiplayerSessionId([6; 32]),
            sequence: 1,
        };
        let mut peers = ServerPeers::new(1);
        peers.snapshot_transition = Some(PendingSnapshotTransition {
            id,
            payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                mission_id: 3,
                save_bytes: vec![4, 5],
            },
            awaiting: HashSet::new(),
        });

        retain_transition_peer_for_reconnect(&mut peers, 1);
        assert!(
            peers
                .snapshot_transition
                .as_ref()
                .unwrap()
                .awaiting
                .contains(&1)
        );
        assert!(take_committed_snapshot_transition(&mut peers).is_none());
    }

    #[test]
    fn reconnect_rejects_wrong_session_mission_config_or_speech_locale() {
        let expected = robin_engine::engine::SimConfig::default();
        let session_id = robin_engine::multiplayer::MultiplayerSessionId([21; 32]);
        assert!(validate_reconnect_session_id(session_id, session_id).is_ok());
        assert!(
            validate_reconnect_session_id(
                session_id,
                robin_engine::multiplayer::MultiplayerSessionId([22; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionB",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );

        let mut changed = expected;
        changed.amount_of_speaking = 9;
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                changed,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(2),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("de-DE"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([2; 32]),
            )
            .is_err()
        );
        assert!(
            validate_reconnect_state(
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
                PlayerId(1),
                "MissionA",
                7,
                expected,
                Some("en-US"),
                robin_engine::multiplayer::MultiplayerSessionId([1; 32]),
            )
            .is_ok()
        );
    }

    #[test]
    fn host_reconnect_directive_ends_the_complete_client_session() {
        let (incoming_tx, _incoming_rx) = std::sync::mpsc::channel();
        let error = handle_client_wire_msg(
            &incoming_tx,
            robin_engine::multiplayer::NetMsg::ReconnectRequired {
                reason: "late input predates rollback horizon".to_string(),
            },
        )
        .expect_err("directive must unwind the session into the reconnect loop");
        assert!(error.contains("full-snapshot reconnect"));
        assert!(error.contains("rollback horizon"));
    }

    #[test]
    fn reconnect_discards_commands_queued_for_abandoned_session() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        sender
            .send(robin_engine::multiplayer::NetOutbound::Input {
                origin_frame: 41,
                command: robin_engine::player_command::PlayerCommand::CrouchDown,
            })
            .expect("queue old-session command");
        sender
            .send(robin_engine::multiplayer::NetOutbound::ReadyToSim { frame: 40 })
            .expect("queue old-session readiness");

        assert_eq!(discard_session_outbound(&mut receiver), 2);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn client_transition_events_preserve_exact_prepare_bytes_and_commit_id() {
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: robin_engine::multiplayer::MultiplayerSessionId([5; 32]),
            sequence: 9,
        };
        let save_bytes = vec![0, 17, 34, 255];
        handle_client_wire_msg(
            &incoming_tx,
            robin_engine::multiplayer::NetMsg::PrepareSnapshotTransition {
                id,
                payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                    mission_id: 71,
                    save_bytes: save_bytes.clone(),
                },
            },
        )
        .unwrap();
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            robin_engine::multiplayer::NetEvent::PrepareSnapshotTransition {
                id: decoded_id,
                payload: robin_engine::multiplayer::SnapshotTransitionPayload::Save {
                    mission_id: 71,
                    save_bytes: decoded_bytes,
                },
            } if decoded_id == id && decoded_bytes == save_bytes
        ));

        handle_client_wire_msg(
            &incoming_tx,
            robin_engine::multiplayer::NetMsg::CommitSnapshotTransition { id },
        )
        .unwrap();
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            robin_engine::multiplayer::NetEvent::CommitSnapshotTransition { id: decoded_id }
                if decoded_id == id
        ));
    }

    #[test]
    fn authenticated_owner_reclaims_and_replaces_only_its_original_seat() {
        let mut peers = ServerPeers::new(3);
        let owner = PeerOwner::Browser([7; 32]);
        let other = PeerOwner::Browser([8; 32]);
        let (first_tx, _first_rx) = unbounded_channel();
        let (seat, generation) = peers.claim_seat(owner, "Robin", first_tx).unwrap();
        assert_eq!(seat, 1);

        let (replacement_tx, _replacement_rx) = unbounded_channel();
        let (replacement_seat, replacement_generation) = peers
            .claim_seat(owner, "Robin renamed", replacement_tx)
            .unwrap();
        assert_eq!(replacement_seat, seat);
        assert_ne!(replacement_generation, generation);
        assert!(!peers.release_seat_if_owner(seat, owner, generation));

        assert!(peers.release_seat_if_owner(seat, owner, replacement_generation));
        let (other_tx, _other_rx) = unbounded_channel();
        let (other_seat, _) = peers.claim_seat(other, "Robin renamed", other_tx).unwrap();
        assert_eq!(
            other_seat, 2,
            "a matching nickname grants no seat authority"
        );

        let (rejoin_tx, _rejoin_rx) = unbounded_channel();
        let (rejoined_seat, _) = peers.claim_seat(owner, "New name", rejoin_tx).unwrap();
        assert_eq!(rejoined_seat, seat);
    }
}
