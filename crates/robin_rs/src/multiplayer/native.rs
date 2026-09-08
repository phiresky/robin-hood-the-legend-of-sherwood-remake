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

use super::client_protocol::{WelcomeData, validate_reconnect_state};
#[cfg(test)]
use super::encode_msg;
use super::identity::{
    GAME_ALPN, bind_endpoint, bind_endpoint_with_relay, game_secret_key, parse_connect_addr,
};
use super::{
    FrameCursor, INPUT_DELAY_FRAMES, InboundFramePolicy, InitialSnapshot,
    MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION, MultiplayerSessionId, NET_PROTOCOL_VERSION,
    NetEvent, NetMsg, NetOutbound, RankedBrowseOnlyReason, RankedJoinAccepted,
    RankedJoinAttestationDocument, RankedJoinChallenge, RankedJoinClaimDocument,
    RankedJoinResponse, RankedParticipantRosterDocument, RankedSessionGenesisDocument,
    SharedClientLeaderboardCoSignState, SharedClientRankedJoinState,
    verify_leaderboard_cosign_response,
};
use crate::distributed_mod::{
    DistributedModPackage, ValidatedDistributedMod, make_distributed_mod_offer,
};
use crate::leaderboard_ranked_session::{
    CampaignContinuationReceiptSelectionRequestV1, CampaignContinuationReceiptSelectionResponseV1,
    OfficialRankedSessionWireSetupV1, RankedSessionClientAdmissionV1, RankedSessionLifecycle,
    SharedRankedSessionLifecycle, decode_ranked_wire_document, encode_ranked_wire_document,
    sign_named_seat_join,
};
use iroh::endpoint::{Connection, ReadExactError, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
// Non-poisoning mutex: a panicking worker must not turn every later
// lock of the shared peer state into a second panic.
use parking_lot::Mutex;
use robin_engine::multiplayer::{
    BrowserPeerAuth, LeaderboardCoSignResponse, browser_seat_proof_message,
};
use robin_engine::player_command::{PlayerCommand, PlayerId, PlayerInput};
use robin_run_protocol::{
    CampaignContinuationPreflightRequestClaimV1, LeaderboardCoSignInstanceV1,
    LeaderboardCoSignRequestV1, ParticipantPublicDisclosureV1, ParticipantSignatureV1, PublicKey32,
    Validate as _,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const RANKED_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
const RANKED_SETUP_AWAITING: u8 = 0;
const RANKED_SETUP_AVAILABLE: u8 = 1;
const RANKED_SETUP_UNAVAILABLE: u8 = 2;

const HANDSHAKE_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
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
    pub fn from_encoded(encoded: Vec<u8>) -> Result<Self, String> {
        let validated = DistributedModPackage::decode(&encoded)
            .map_err(|error| format!("validate hosted distributed mod: {error}"))?;
        Ok(Self {
            validated,
            encoded: Arc::from(encoded),
        })
    }

    fn offer(
        &self,
        host_endpoint_id: String,
    ) -> Result<robin_engine::multiplayer::DistributedModOffer, String> {
        make_distributed_mod_offer(&self.validated, self.encoded.len() as u64, host_endpoint_id)
            .map_err(|error| format!("build distributed-mod offer: {error}"))
    }
}

/// Explicit campaign lifetime, independent of each mission's QUIC endpoint.
/// Transport credentials are deliberately neither persisted nor reconstructed
/// by deserialization. Durable ranked identity remains install-owned.
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

    pub(crate) fn discard_host_continuation(&self) -> Result<(), String> {
        let _lease = self.reserve_server().map_err(|error| error.to_string())?;
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
) -> Result<Option<HostSessionContinuation>, String> {
    let slot = state.continuation.lock();
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
    Ok(slot.clone())
}

// ─── Framing ─────────────────────────────────────────────────────

async fn write_frame(send: &mut SendStream, msg: &NetMsg) -> Result<(), String> {
    let (header, bytes) = super::client_protocol::encode_frame(msg)?;
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
    let (class, len) = super::client_protocol::decode_header(header, policy)?;
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| format!("read frame body: {e}"))?;
    super::client_protocol::decode_body(class, &buf).map(Some)
}

async fn read_frame_bounded_with_timeout(
    recv: &mut RecvStream,
    policy: InboundFramePolicy,
    timeout: Duration,
    phase: &str,
) -> Result<Option<NetMsg>, String> {
    tokio::time::timeout(timeout, read_frame(recv, policy))
        .await
        .map_err(|_| format!("{phase} timed out after {timeout:?}"))?
}

async fn write_frame_with_timeout(
    send: &mut SendStream,
    msg: &NetMsg,
    timeout: Duration,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(timeout, write_frame(send, msg))
        .await
        .map_err(|_| format!("{phase} timed out after {timeout:?}"))?
}

fn checked_epoch_ms(millis: u128) -> Result<u64, String> {
    u64::try_from(millis)
        .map_err(|_| "system clock timestamp exceeds the u64 Unix range".to_owned())
}

fn current_epoch_ms() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes the Unix epoch: {error}"))?;
    checked_epoch_ms(duration.as_millis())
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
/// Drop the handle before creating the next mission transport: retained
/// handles keep the campaign lease so they cannot publish stale handoffs.
pub struct ServerHandle {
    /// `(local_seat, mission_seed)` the server is operating with.
    pub local_seat: PlayerId,
    pub mission_seed: u64,
    session_id: MultiplayerSessionId,
    endpoint_id: EndpointId,
    endpoint_addr: EndpointAddr,
    host_key: SecretKey,
    mission_id: String,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    context: Arc<ServerContext>,
    preserve_on_shutdown: bool,
    cancellation: Arc<AtomicBool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    runtime_thread: Option<JoinHandle<()>>,
    bridge_thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    pub fn session_id(&self) -> MultiplayerSessionId {
        self.session_id
    }

    pub(crate) fn ranked_lifecycle(&self) -> SharedRankedSessionLifecycle {
        Arc::clone(&self.ranked_lifecycle)
    }

    pub(crate) fn ranked_local_seat(&self) -> Result<PlayerId, String> {
        Ok(self.local_seat)
    }

    pub(crate) fn ranked_host_public_key(&self) -> PublicKey32 {
        PublicKey32::from_bytes(*self.host_key.public().as_bytes())
    }

    pub(crate) fn ranked_authenticated_seats(&self) -> Vec<(PlayerId, PublicKey32)> {
        self.context
            .peers
            .lock()
            .ranked_identities
            .iter()
            .filter_map(|(seat, identity)| {
                identity
                    .durable_public_key
                    .map(|key| (PlayerId(*seat), PublicKey32::from_bytes(key)))
            })
            .chain(std::iter::once((
                PlayerId::HOST,
                PublicKey32::from_bytes(*self.host_key.public().as_bytes()),
            )))
            .collect()
    }

    pub(crate) fn ranked_preflight_lobby(
        &self,
    ) -> Option<crate::leaderboard_ranked_session::RankedPreflightLobbyV1> {
        let peers = self.context.peers.lock();
        let max_concurrent_players = u16::try_from(peers.expected_players).ok()?;
        let mut participant_public_keys = peers
            .ranked_identities
            .values()
            .map(|identity| identity.durable_public_key.map(PublicKey32::from_bytes))
            .collect::<Option<Vec<_>>>()?;
        participant_public_keys.push(PublicKey32::from_bytes(*self.host_key.public().as_bytes()));
        participant_public_keys.sort_unstable();
        participant_public_keys.dedup();
        if participant_public_keys.len() != usize::from(max_concurrent_players) {
            return None;
        }
        Some(crate::leaderboard_ranked_session::RankedPreflightLobbyV1 {
            host_public_key: PublicKey32::from_bytes(*self.host_key.public().as_bytes()),
            max_concurrent_players,
            participant_public_keys,
        })
    }

    pub(crate) fn install_ranked_session_setup(
        &self,
        setup: Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        let Some(setup) = setup else {
            downgrade_ranked_session(
                &self.context,
                RankedBrowseOnlyReason::HostRankedSessionUnavailable,
                "host prepared inputs explicitly selected browse-only multiplayer",
            );
            return Ok(());
        };
        let ranked_host_key = ed25519_dalek::SigningKey::from_bytes(&self.host_key.to_bytes());
        let session = crate::leaderboard_ranked_session::RankedSessionHost::new_official(
            &ranked_host_key,
            NET_PROTOCOL_VERSION,
            setup,
        )
        .map_err(|error| format!("construct official ranked host session: {error}"))?;
        ranked_lifecycle_lock(&self.ranked_lifecycle)
            .install_ranked(session)
            .map_err(|error| format!("install ranked host session: {error}"))?;
        progress_ranked_admission(&self.context);
        Ok(())
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
            current_epoch_ms()? / 1000,
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
        if let Some(handle) = self.bridge_thread.take()
            && handle.join().is_err()
        {
            tracing::error!("multiplayer server outgoing bridge panicked during shutdown");
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
    publish_host_session_continuation(
        &context.campaign,
        HostSessionContinuation {
            host_endpoint_id: context.host_endpoint_id,
            session_id: context.session_id,
            expected_players: peers.expected_players,
            owner_seats,
            relay_url: context.relay_url.lock().clone(),
        },
    );
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
    /// Seats whose deterministic `ConnectSeat` has been published. A QUIC
    /// stream may exist before this while ranked admission is pending.
    sim_connected_seats: HashSet<u8>,
    /// Presentation names per active seat. Names never grant authority.
    nicknames: HashMap<u8, String>,
    /// Durable authenticated owner per seat. Native identities are the QUIC
    /// endpoint key; browser identities are independently signed durable keys.
    owners: HashMap<u8, PeerOwner>,
    ranked_identities: HashMap<u8, RankedPeerIdentity>,
    seat_claim_kinds: HashMap<u8, SeatClaimKind>,
    disconnected_seats: HashMap<PeerOwner, u8>,
    session_generations: HashMap<u8, u64>,
    next_session_generation: u64,
    expected_players: u32,
    host_ready_frame: Option<u32>,
    ready_seats: HashMap<u8, u32>,
    begin_sent: Option<(u32, u64)>,
    snapshot_transition: Option<PendingSnapshotTransition>,
    leaderboard_cosign: Vec<PendingLeaderboardCoSign>,
    leaderboard_cosign_seen: Vec<(LeaderboardCoSignInstanceV1, u8)>,
    pending_ranked_admission: Option<PendingRankedAdmission>,
}

struct PendingSnapshotTransition {
    id: robin_engine::multiplayer::SnapshotTransitionId,
    payload: robin_engine::multiplayer::SnapshotTransitionPayload,
    awaiting: HashSet<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingLeaderboardCoSign {
    target_seat: u8,
    request: LeaderboardCoSignRequestV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SeatClaimKind {
    Fresh,
    Reconnect,
    ActiveReplacement,
}

#[derive(Debug)]
struct SeatClaim {
    seat: u8,
    generation: u64,
    kind: SeatClaimKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RankedPeerIdentity {
    durable_public_key: Option<[u8; 32]>,
    transport_endpoint_id: [u8; 32],
    public_disclosure: ParticipantPublicDisclosureV1,
}

#[derive(Clone, Debug)]
struct PendingRankedAdmission {
    seat: u8,
    generation: u64,
    kind: RankedAdmissionKind,
    challenge: RankedJoinChallenge,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RankedAdmissionKind {
    Fresh,
    Reconnect,
}

impl ServerPeers {
    fn new(expected_players: u32) -> Self {
        Self {
            next_seat: 1,
            senders: HashMap::new(),
            sim_connected_seats: HashSet::new(),
            nicknames: HashMap::new(),
            owners: HashMap::new(),
            ranked_identities: HashMap::new(),
            seat_claim_kinds: HashMap::new(),
            disconnected_seats: HashMap::new(),
            session_generations: HashMap::new(),
            next_session_generation: 1,
            expected_players,
            host_ready_frame: None,
            ready_seats: HashMap::new(),
            begin_sent: None,
            snapshot_transition: None,
            leaderboard_cosign: Vec::new(),
            leaderboard_cosign_seen: Vec::new(),
            pending_ranked_admission: None,
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
            sim_connected_seats: HashSet::new(),
            nicknames: HashMap::new(),
            owners: HashMap::new(),
            ranked_identities: HashMap::new(),
            seat_claim_kinds: HashMap::new(),
            disconnected_seats: continuation.owner_seats.clone(),
            session_generations: HashMap::new(),
            next_session_generation: 1,
            expected_players: continuation.expected_players,
            host_ready_frame: None,
            ready_seats: HashMap::new(),
            begin_sent: None,
            snapshot_transition: None,
            leaderboard_cosign: Vec::new(),
            leaderboard_cosign_seen: Vec::new(),
            pending_ranked_admission: None,
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
        ranked_identity: RankedPeerIdentity,
        sender: UnboundedSender<NetMsg>,
    ) -> Result<SeatClaim, String> {
        let (seat, kind) = if let Some(active) = self
            .owners
            .iter()
            .find_map(|(&seat, active_owner)| (*active_owner == owner).then_some(seat))
        {
            self.disconnected_seats.remove(&owner);
            (active, SeatClaimKind::ActiveReplacement)
        } else if let Some(disconnected) = self.disconnected_seats.remove(&owner) {
            (disconnected, SeatClaimKind::Reconnect)
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
            (next, SeatClaimKind::Fresh)
        };
        let generation = self.next_session_generation;
        self.next_session_generation = generation
            .checked_add(1)
            .ok_or_else(|| "multiplayer session generation overflow".to_string())?;
        self.senders.insert(seat, sender);
        self.nicknames.insert(seat, nickname.to_owned());
        self.owners.insert(seat, owner);
        self.ranked_identities.insert(seat, ranked_identity);
        self.seat_claim_kinds.insert(seat, kind);
        self.session_generations.insert(seat, generation);
        self.ready_seats.remove(&seat);
        Ok(SeatClaim {
            seat,
            generation,
            kind,
        })
    }

    fn release_seat_if_owner(
        &mut self,
        seat: u8,
        owner: PeerOwner,
        generation: u64,
    ) -> Option<bool> {
        if self.session_generations.get(&seat) != Some(&generation)
            || self.owners.get(&seat) != Some(&owner)
        {
            return None;
        }
        self.senders.remove(&seat);
        self.nicknames.remove(&seat).unwrap_or_else(|| {
            panic!("authenticated active multiplayer seat {seat} has no nickname")
        });
        self.owners
            .remove(&seat)
            .unwrap_or_else(|| panic!("authenticated active multiplayer seat {seat} has no owner"));
        self.ranked_identities.remove(&seat).unwrap_or_else(|| {
            panic!("authenticated active multiplayer seat {seat} has no ranked identity metadata")
        });
        self.seat_claim_kinds.remove(&seat).unwrap_or_else(|| {
            panic!("authenticated active multiplayer seat {seat} has no claim classification")
        });
        self.session_generations.remove(&seat);
        self.ready_seats.remove(&seat);
        self.disconnected_seats.insert(owner, seat);
        Some(self.sim_connected_seats.remove(&seat))
    }

    /// Atomically retain the exact request before returning its one target's
    /// queue. This ordering ensures even an immediate peer response always
    /// finds pending authenticated server state.
    fn begin_leaderboard_cosign(
        &mut self,
        target: PlayerId,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<UnboundedSender<NetMsg>, String> {
        if target == PlayerId::HOST {
            return Err("leaderboard co-sign requests to the host must be signed locally".into());
        }
        request
            .signing_bytes()
            .map_err(|error| format!("invalid leaderboard co-sign request: {error}"))?;
        if self.leaderboard_cosign_seen.len() >= MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION {
            return Err(format!(
                "leaderboard co-sign request history exceeds the per-session limit of {}",
                MAX_LEADERBOARD_COSIGN_REQUESTS_PER_SESSION
            ));
        }
        let key = (request.instance, target.0);
        if self.leaderboard_cosign_seen.contains(&key) {
            return Err(format!(
                "duplicate leaderboard co-sign request instance for target {target:?}"
            ));
        }
        let sender = self.senders.get(&target.0).cloned().ok_or_else(|| {
            format!("leaderboard co-sign target {target:?} is not an authenticated active peer")
        })?;
        self.leaderboard_cosign_seen.push(key);
        self.leaderboard_cosign.push(PendingLeaderboardCoSign {
            target_seat: target.0,
            request,
        });
        Ok(sender)
    }

    /// Verify and consume exactly one targeted request. The caller-supplied
    /// seat is stamped by the authenticated stream and never read from the
    /// response payload.
    fn complete_leaderboard_cosign(
        &mut self,
        from: PlayerId,
        response: &LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let position = self
            .leaderboard_cosign
            .iter()
            .position(|pending| {
                pending.target_seat == from.0 && pending.request.instance == response.instance
            })
            .ok_or_else(|| {
                format!(
                    "peer {from:?} submitted a duplicate, wrong-target, or wrong-session leaderboard co-sign response"
                )
            })?;
        let request = self.leaderboard_cosign[position].request;
        let expected_signer = self
            .ranked_identities
            .get(&from.0)
            .and_then(|identity| identity.durable_public_key)
            .ok_or_else(|| {
                format!(
                    "peer {from:?} has no admitted durable ranked identity for leaderboard co-signing"
                )
            })?;
        if response.signer_public_key != expected_signer {
            return Err(format!(
                "peer {from:?} signed a leaderboard request with a key other than its admitted durable identity"
            ));
        }
        verify_leaderboard_cosign_response(&request, response)?;
        self.leaderboard_cosign.remove(position);
        Ok(())
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
) -> Result<Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>, String> {
    if peers.begin_sent.is_some() {
        return Ok(None);
    }
    let Some(host_frame) = peers.host_ready_frame else {
        return Ok(None);
    };
    let active_peer_count = peers.sim_connected_seats.len() as u32;
    let expected_peer_count = peers.expected_players.saturating_sub(1);
    if active_peer_count < expected_peer_count {
        return Ok(None);
    }
    if !peers
        .sim_connected_seats
        .iter()
        .all(|seat| peers.senders.contains_key(seat) && peers.ready_seats.contains_key(seat))
    {
        return Ok(None);
    }

    let begin_frame = peers
        .ready_seats
        .values()
        .copied()
        .fold(host_frame, u32::max);
    let start_epoch_ms = current_epoch_ms()?
        .checked_add(500)
        .ok_or_else(|| "multiplayer BeginSim timestamp exceeds the u64 Unix range".to_owned())?;
    let senders = peers.senders.values().cloned().collect();
    peers.begin_sent = Some((begin_frame, start_epoch_ms));
    Ok(Some((begin_frame, start_epoch_ms, senders)))
}

/// Per-session context shared by every server task.
struct ServerContext {
    _campaign_lease: CampaignServerLease,
    campaign: Arc<CampaignTransportState>,
    peers: Mutex<ServerPeers>,
    incoming_tx: Sender<NetEvent>,
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    host_endpoint_id: EndpointId,
    session_id: MultiplayerSessionId,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_browse_reason: Mutex<Option<RankedBrowseOnlyReason>>,
    continued_session: bool,
    relay_url: Mutex<Option<iroh::RelayUrl>>,
    speech_timing_locale: Option<String>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    content: Option<HostedModContent>,
    cancellation: Arc<AtomicBool>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

fn fail_server(context: &ServerContext, error: String) {
    if !context.cancellation.swap(true, Ordering::AcqRel) {
        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
    }
    let _ = context.shutdown_tx.send(true);
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
        &MultiplayerCampaignSession::default(),
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
        None,
        browser_join_enabled,
    )
}

/// Start a server which admits the exact complete mod before Welcome.
#[allow(clippy::too_many_arguments)]
pub fn start_server_with_content(
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
    content: HostedModContent,
    browser_join_enabled: bool,
) -> std::io::Result<ServerHandle> {
    let key = game_secret_key().map_err(std::io::Error::other)?;
    start_server_inner(
        &MultiplayerCampaignSession::default(),
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
        Some(content),
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
        &MultiplayerCampaignSession::default(),
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
        None,
        false,
    )
}

/// Test-only explicit-key entry point for an exact hosted package. Browser
/// ticket publication is disabled; production hosting uses
/// [`start_server_with_content`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(super) fn start_server_with_key_and_content(
    key: SecretKey,
    host_nickname: String,
    mission_id: String,
    mission_seed: u64,
    sim_config: robin_engine::engine::SimConfig,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    expected_players: u32,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        &MultiplayerCampaignSession::default(),
        key,
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        None,
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        expected_players,
        content,
        false,
    )
}

/// Start a mission transport within an explicitly owned campaign.
#[allow(clippy::too_many_arguments)]
pub fn start_server_in_campaign(
    campaign: &MultiplayerCampaignSession,
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
    content: Option<HostedModContent>,
    browser_join_enabled: bool,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        campaign,
        game_secret_key().map_err(std::io::Error::other)?,
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
        content,
        browser_join_enabled,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_server_inner(
    campaign: &MultiplayerCampaignSession,
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
    content: Option<HostedModContent>,
    browser_join_enabled: bool,
) -> std::io::Result<ServerHandle> {
    let campaign_lease = campaign.reserve_server()?;
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
    if let Some(content) = &content {
        if content.validated.package.manifest.mission_basename != mission_id {
            return Err(std::io::Error::other(format!(
                "hosted full mod contains mission `{}`, but server announces `{mission_id}`",
                content.validated.package.manifest.mission_basename
            )));
        }
        content
            .offer(host_endpoint_id.to_string())
            .map_err(std::io::Error::other)?;
    }
    let continuation =
        pending_host_session_continuation(campaign.state(), host_endpoint_id, expected_players)
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
        _campaign_lease: campaign_lease,
        campaign: Arc::clone(campaign.state()),
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
        ranked_lifecycle: Arc::new(std::sync::Mutex::new(
            RankedSessionLifecycle::awaiting_prepared_inputs(),
        )),
        ranked_browse_reason: Mutex::new(None),
        continued_session: continuation.is_some(),
        relay_url: Mutex::new(
            continuation
                .as_ref()
                .and_then(|continuation| continuation.relay_url.clone()),
        ),
        speech_timing_locale,
        frame_cursor,
        initial_snapshot,
        content,
        cancellation: Arc::clone(&cancellation),
        shutdown_tx: shutdown_tx.clone(),
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
                    context.cancellation.store(true, Ordering::Release);
                    let _ = startup_tx.send(Err(format!("build tokio runtime: {e}")));
                    return;
                }
            };
            rt.block_on(run_server(
                key,
                Arc::clone(&context),
                outgoing_async_rx,
                startup_tx,
                shutdown_rx,
                browser_join_enabled,
            ));
            context.cancellation.store(true, Ordering::Release);
        }
    });
    let runtime_thread = match runtime_thread {
        Ok(handle) => handle,
        Err(error) => {
            cancellation.store(true, Ordering::Release);
            if bridge_thread.join().is_err() {
                tracing::error!("multiplayer server outgoing bridge panicked after failed startup");
            }
            return Err(error);
        }
    };

    let (endpoint_id, endpoint_addr) = match startup_rx.recv() {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            let _ = bridge_thread.join();
            return Err(std::io::Error::other(err));
        }
        Err(e) => {
            cancellation.store(true, Ordering::Release);
            let _ = shutdown_tx.send(true);
            let _ = runtime_thread.join();
            let _ = bridge_thread.join();
            return Err(std::io::Error::other(format!(
                "server startup channel closed: {e}"
            )));
        }
    };
    // Only successful startup consumes the handoff. Validation, thread or
    // endpoint failures leave it available for an explicit retry.
    *campaign.state().continuation.lock() = None;
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
        ranked_lifecycle: Arc::clone(&context.ranked_lifecycle),
        context,
        preserve_on_shutdown: false,
        cancellation,
        shutdown_tx,
        runtime_thread: Some(runtime_thread),
        bridge_thread: Some(bridge_thread),
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

    let mut pump = tokio::spawn(run_server_outgoing_pump(
        Arc::clone(&context),
        outgoing_async_rx,
    ));
    let accept = tokio::spawn(run_server_accept_loop(
        Arc::clone(&context),
        endpoint.clone(),
    ));

    // Root task: explicit shutdown and an invalid host-side publication both
    // close the endpoint, ending every peer connection and the accept loop.
    // A direction violation is surfaced to the game instead of leaving a
    // partially live server with a dead outgoing pump.
    let pump_result = tokio::select! {
        _ = shutdown_rx.wait_for(|stop| *stop) => None,
        result = &mut pump => Some(result),
    };
    let pump_finished = pump_result.is_some();
    if let Some(result) = pump_result {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                context.cancellation.store(true, Ordering::Release);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            Err(error) => {
                context.cancellation.store(true, Ordering::Release);
                let _ = context.incoming_tx.send(NetEvent::Fatal(format!(
                    "multiplayer server outgoing pump failed: {error}"
                )));
            }
        }
    }
    endpoint.close().await;
    accept.abort();
    let _ = accept.await;
    if !pump_finished {
        pump.abort();
        let _ = pump.await;
    }
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
) -> Result<(), String> {
    while let Some(msg) = outgoing_async_rx.recv().await {
        validate_server_gameplay_outbound(&msg)?;
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
                resolve_ranked_before_ready(&context);
                let begin = {
                    let mut p = context.peers.lock();
                    p.host_ready_frame = Some(frame);
                    maybe_begin_sim_locked(&mut p)
                }?;
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
            NetOutbound::RankedBrowseOnly { reason } => {
                downgrade_ranked_session(
                    &context,
                    reason,
                    "host runtime explicitly resolved this multiplayer session as browse-only",
                );
            }
            NetOutbound::RankedOfficialSessionSetup(setup) => {
                if let Err(error) =
                    crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                        OfficialRankedSessionWireSetupV1,
                    >(setup.as_bytes())
                {
                    let error =
                        format!("host rejected invalid official ranked wire setup: {error}");
                    tracing::error!(%error);
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    continue;
                }
                if let Err(error) =
                    broadcast_msg_required(&context, NetMsg::RankedOfficialSessionSetup(setup))
                {
                    tracing::error!(%error, "official ranked setup broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::RankedContinuationReceiptSelectionRequest(request) => {
                if let Err(error) = decode_ranked_wire_document::<
                    CampaignContinuationReceiptSelectionRequestV1,
                >(request.as_bytes())
                {
                    let error = format!(
                        "host rejected invalid continuation receipt selection request: {error}"
                    );
                    tracing::error!(%error);
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    continue;
                }
                if let Err(error) = broadcast_msg_required(
                    &context,
                    NetMsg::RankedContinuationReceiptSelectionRequest(request),
                ) {
                    tracing::error!(%error, "continuation receipt selection broadcast failed");
                    let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                }
            }
            NetOutbound::RankedContinuationReceiptSelection(_) => {
                let error = "multiplayer host attempted to send a client-only continuation receipt selection".to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::RankedContinuationPreflightClaim { to, claim } => {
                let decoded = decode_ranked_wire_document::<
                    CampaignContinuationPreflightRequestClaimV1,
                >(claim.as_bytes());
                let claim_document = match decoded {
                    Ok(document) => document,
                    Err(error) => {
                        let error = format!(
                            "host rejected invalid continuation preflight claim for {to:?}: {error}"
                        );
                        tracing::error!(%error);
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        continue;
                    }
                };
                let sender = {
                    let peers = context.peers.lock();
                    let expected_controller = peers
                        .ranked_identities
                        .get(&to.0)
                        .and_then(|identity| identity.durable_public_key)
                        .map(PublicKey32::from_bytes);
                    if to == PlayerId::HOST
                        || expected_controller
                            != Some(claim_document.campaign_controller_public_key)
                    {
                        None
                    } else {
                        peers.senders.get(&to.0).cloned()
                    }
                };
                match sender {
                    Some(sender) => {
                        if sender
                            .send(NetMsg::RankedContinuationPreflightClaim(claim))
                            .is_err()
                        {
                            let error = format!(
                                "continuation preflight controller {to:?} disconnected before claim delivery"
                            );
                            tracing::error!(%error);
                            let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        }
                    }
                    None => {
                        let error = format!(
                            "continuation preflight target {to:?} is not the authenticated controller"
                        );
                        tracing::error!(%error);
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    }
                }
            }
            NetOutbound::RankedContinuationPreflightSignature(_) => {
                let error = "multiplayer host attempted to send a client-only continuation preflight signature".to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::RankedCoSignContext {
                to,
                context: document,
            } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored ranked co-sign context after eligibility ended"
                    );
                    continue;
                }
                if let Err(error) = decode_ranked_wire_document::<
                    crate::leaderboard_ranked_session::RankedCoSignContextV1,
                >(document.as_bytes())
                {
                    tracing::error!(%error, ?to, "rejected invalid ranked co-sign context");
                    continue;
                }
                let sender = {
                    let peers = context.peers.lock();
                    peers
                        .sim_connected_seats
                        .contains(&to.0)
                        .then(|| peers.senders.get(&to.0).cloned())
                        .flatten()
                };
                match sender {
                    Some(sender) => {
                        if sender.send(NetMsg::RankedCoSignContext(document)).is_err() {
                            tracing::warn!(?to, "ranked co-sign context target disconnected");
                        }
                    }
                    None => tracing::warn!(?to, "ranked co-sign context target is not admitted"),
                }
            }
            NetOutbound::RankedSubmissionAccepted { to, accepted } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored ranked submission acknowledgement after eligibility ended"
                    );
                    continue;
                }
                if let Err(error) = decode_ranked_wire_document::<
                    robin_run_protocol::SubmissionAcceptedV1,
                >(accepted.as_bytes())
                {
                    tracing::error!(%error, ?to, "rejected invalid ranked submission acknowledgement");
                    continue;
                }
                let sender = {
                    let peers = context.peers.lock();
                    peers
                        .sim_connected_seats
                        .contains(&to.0)
                        .then(|| peers.senders.get(&to.0).cloned())
                        .flatten()
                };
                match sender {
                    Some(sender) => {
                        if sender
                            .send(NetMsg::RankedSubmissionAccepted(accepted))
                            .is_err()
                        {
                            tracing::warn!(
                                ?to,
                                "ranked submission acknowledgement target disconnected"
                            );
                        }
                    }
                    None => tracing::warn!(
                        ?to,
                        "ranked submission acknowledgement target is not admitted"
                    ),
                }
            }
            NetOutbound::RankedJoinChallenge { .. }
            | NetOutbound::RankedJoinAccepted { .. }
            | NetOutbound::RankedParticipantRoster { .. }
            | NetOutbound::ArmRankedJoin { .. }
            | NetOutbound::RankedJoinResponse(_) => {
                tracing::error!(
                    "native host ignored an externally-authored ranked admission control"
                );
            }
            NetOutbound::LeaderboardCoSignRequest { to, request } => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?to,
                        "ignored leaderboard co-sign request after ranked eligibility ended"
                    );
                    continue;
                }
                let sender = {
                    let mut peers = context.peers.lock();
                    peers.begin_leaderboard_cosign(to, request)
                };
                match sender {
                    Ok(sender) => {
                        if sender
                            .send(NetMsg::LeaderboardCoSignRequest(request))
                            .is_err()
                        {
                            let error = format!(
                                "authenticated leaderboard co-sign target {to:?} closed before request delivery"
                            );
                            tracing::error!(%error);
                            let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "leaderboard co-sign request rejected");
                        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
                    }
                }
            }
            NetOutbound::ArmLeaderboardCoSignRequest { .. } => {
                let error =
                    "multiplayer host attempted to arm a client-only leaderboard co-sign request"
                        .to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::LeaderboardCoSignResponse(_) => {
                let error =
                    "multiplayer host attempted to send a client-only leaderboard co-sign response"
                        .to_string();
                tracing::error!(%error);
                let _ = context.incoming_tx.send(NetEvent::Fatal(error));
            }
            NetOutbound::ContentRequest { .. }
            | NetOutbound::ContentReject { .. }
            | NetOutbound::ContentReady { .. }
            | NetOutbound::ContentPrepared { .. } => {
                unreachable!("server gameplay outbound was validated before dispatch")
            }
        }
    }
    tracing::info!("server outgoing pump stopped");
    Ok(())
}

fn validate_server_gameplay_outbound(outgoing: &NetOutbound) -> Result<(), String> {
    match outgoing {
        NetOutbound::Input { .. }
        | NetOutbound::StateHash { .. }
        | NetOutbound::InitialSnapshot { .. }
        | NetOutbound::ReadyToSim { .. }
        | NetOutbound::ModalDecision { .. }
        | NetOutbound::ReconnectForSnapshot { .. }
        | NetOutbound::ReconnectAllForSnapshot { .. }
        | NetOutbound::BeginSnapshotTransition { .. }
        | NetOutbound::RankedBrowseOnly { .. }
        | NetOutbound::RankedOfficialSessionSetup(_)
        | NetOutbound::RankedContinuationReceiptSelectionRequest(_)
        | NetOutbound::RankedContinuationPreflightClaim { .. }
        | NetOutbound::RankedCoSignContext { .. }
        | NetOutbound::RankedSubmissionAccepted { .. }
        | NetOutbound::RankedJoinChallenge { .. }
        | NetOutbound::RankedJoinAccepted { .. }
        | NetOutbound::RankedParticipantRoster { .. }
        | NetOutbound::LeaderboardCoSignRequest { .. } => Ok(()),
        NetOutbound::ModalProposal { .. }
        | NetOutbound::SnapshotTransitionReady { .. }
        | NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. }
        | NetOutbound::RankedContinuationReceiptSelection(_)
        | NetOutbound::RankedContinuationPreflightSignature(_)
        | NetOutbound::ArmRankedJoin { .. }
        | NetOutbound::RankedJoinResponse(_)
        | NetOutbound::ArmLeaderboardCoSignRequest { .. }
        | NetOutbound::LeaderboardCoSignResponse(_) => {
            Err("multiplayer host queued a client-only output".to_owned())
        }
    }
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

fn ranked_lifecycle_lock(
    lifecycle: &SharedRankedSessionLifecycle,
) -> std::sync::MutexGuard<'_, RankedSessionLifecycle> {
    lifecycle.lock().unwrap_or_else(|poisoned| {
        tracing::error!(
            "ranked session lifecycle lock was poisoned; retaining authoritative state"
        );
        poisoned.into_inner()
    })
}

fn validate_official_ranked_session(
    context: &ServerContext,
    session: &crate::leaderboard_ranked_session::RankedSessionHost,
) -> Result<(), String> {
    let genesis = session.genesis();
    let ranked = &genesis.claim.ranked_session;
    if genesis.claim.network_protocol_version != NET_PROTOCOL_VERSION
        || *genesis.claim.host_public_key.as_bytes() != *context.host_endpoint_id.as_bytes()
        || ranked.mission_id != context.mission_id
        || ranked.simulation_seed.get() != context.mission_seed
        || ranked.spellforge_content_sha256.is_some()
        || !robin_run_protocol::official_content_subjects_v1(ranked.content_edition)
            .contains(&ranked.content_subject)
    {
        return Err(
            "ranked genesis is not the exact accepted official mission/content policy".to_string(),
        );
    }
    Ok(())
}

fn publish_connect_seats(context: &ServerContext, seats: Vec<(u8, String)>) {
    for (seat, nickname) in seats {
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        broadcast_input(
            context,
            now,
            now,
            target,
            PlayerInput::new(
                PlayerId::HOST,
                PlayerCommand::ConnectSeat {
                    player_id: PlayerId(seat),
                    nickname,
                },
            ),
        );
    }
}

fn retry_begin_sim_after_ranked_resolution(context: &ServerContext) {
    let begin = {
        let mut peers = context.peers.lock();
        maybe_begin_sim_locked(&mut peers)
    };
    match begin {
        Ok(begin) => announce_begin_sim(context, begin),
        Err(error) => {
            tracing::error!(%error, "multiplayer ready barrier could not produce a start time");
            fail_server(context, error);
        }
    }
}

fn finish_ranked_seat_connections(context: &ServerContext, seats: &[u8]) {
    let cached_begin = {
        let peers = context.peers.lock();
        peers.begin_sent.map(|(frame, start_epoch_ms)| {
            let senders = seats
                .iter()
                .filter_map(|seat| peers.senders.get(seat).cloned())
                .collect::<Vec<_>>();
            (frame, start_epoch_ms, senders)
        })
    };
    let Some((frame, start_epoch_ms, senders)) = cached_begin else {
        retry_begin_sim_after_ranked_resolution(context);
        return;
    };
    let snapshot_frame = context
        .initial_snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|(frame, _)| *frame);
    let begin_frame = snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
    let begin_start_epoch_ms = if begin_frame != frame {
        match current_epoch_ms().and_then(|now| {
            now.checked_add(100).ok_or_else(|| {
                "multiplayer ranked preflight timestamp exceeds the u64 Unix range".to_owned()
            })
        }) {
            Ok(start_epoch_ms) => start_epoch_ms,
            Err(error) => {
                tracing::error!(%error, "ranked preflight could not produce a start time");
                fail_server(context, error);
                return;
            }
        }
    } else {
        start_epoch_ms
    };
    for sender in senders {
        let _ = sender.send(NetMsg::BeginSim {
            frame: begin_frame,
            start_epoch_ms: begin_start_epoch_ms,
        });
    }
}

fn connect_all_provisional_seats(context: &ServerContext) {
    let seats = {
        let mut peers = context.peers.lock();
        let mut seats = peers
            .senders
            .keys()
            .copied()
            .filter(|seat| !peers.sim_connected_seats.contains(seat))
            .collect::<Vec<_>>();
        seats.sort_unstable();
        seats
            .into_iter()
            .map(|seat| {
                let nickname = peers.nicknames.get(&seat).cloned().unwrap_or_else(|| {
                    panic!("authenticated provisional seat {seat} has no nickname")
                });
                assert!(peers.sim_connected_seats.insert(seat));
                (seat, nickname)
            })
            .collect::<Vec<_>>()
    };
    let connected_seats = seats.iter().map(|(seat, _)| *seat).collect::<Vec<_>>();
    publish_connect_seats(context, seats);
    finish_ranked_seat_connections(context, &connected_seats);
}

fn downgrade_ranked_session(
    context: &ServerContext,
    wire_reason: RankedBrowseOnlyReason,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    let transitioned = {
        let mut lifecycle = ranked_lifecycle_lock(&context.ranked_lifecycle);
        let transitioned = lifecycle.browse_only_reason().is_none();
        if let Some(session) = lifecycle.ranked_mut() {
            session.cancel_pending_join();
        }
        lifecycle.downgrade(detail.clone());
        transitioned
    };
    context.peers.lock().pending_ranked_admission = None;
    if transitioned {
        *context.ranked_browse_reason.lock() = Some(wire_reason);
        tracing::warn!(reason = ?wire_reason, %detail, "ranked multiplayer downgraded; gameplay remains available");
        broadcast_msg(
            context,
            NetMsg::RankedBrowseOnly {
                reason: wire_reason,
            },
        );
        let _ = context.incoming_tx.send(NetEvent::RankedBrowseOnly {
            reason: wire_reason,
        });
    }
    connect_all_provisional_seats(context);
}

enum RankedAdmissionProgress {
    Idle,
    BrowseOnly,
    Challenge {
        sender: UnboundedSender<NetMsg>,
        challenge: RankedJoinChallenge,
    },
    Downgrade {
        reason: RankedBrowseOnlyReason,
        detail: String,
    },
}

fn progress_ranked_admission(context: &ServerContext) {
    loop {
        let progress = {
            let mut lifecycle = ranked_lifecycle_lock(&context.ranked_lifecycle);
            if lifecycle.is_awaiting_prepared_inputs() {
                RankedAdmissionProgress::Idle
            } else if lifecycle.browse_only_reason().is_some() {
                RankedAdmissionProgress::BrowseOnly
            } else {
                let session = lifecycle
                    .ranked_mut()
                    .expect("resolved non-browse ranked lifecycle has host state");
                if let Err(detail) = validate_official_ranked_session(context, session) {
                    RankedAdmissionProgress::Downgrade {
                        reason: RankedBrowseOnlyReason::HostRankedSessionUnavailable,
                        detail,
                    }
                } else {
                    let mut peers = context.peers.lock();
                    if let Some(pending) = peers.pending_ranked_admission.as_ref()
                        && (peers.session_generations.get(&pending.seat)
                            != Some(&pending.generation)
                            || !peers.senders.contains_key(&pending.seat))
                    {
                        session.cancel_pending_join();
                        peers.pending_ranked_admission = None;
                    }
                    if peers.pending_ranked_admission.is_some() {
                        RankedAdmissionProgress::Idle
                    } else {
                        let next_seat = peers
                            .senders
                            .keys()
                            .copied()
                            .filter(|seat| !peers.sim_connected_seats.contains(seat))
                            .min();
                        let Some(seat) = next_seat else {
                            return;
                        };
                        let identity = *peers.ranked_identities.get(&seat).unwrap_or_else(|| {
                            panic!("authenticated provisional seat {seat} has no ranked identity")
                        });
                        if let Some(durable_public_key) = identity.durable_public_key {
                            let participant_exists = session
                                .participant_claims()
                                .iter()
                                .any(|participant| participant.seat == u16::from(seat));
                            let kind = if participant_exists {
                                RankedAdmissionKind::Reconnect
                            } else {
                                RankedAdmissionKind::Fresh
                            };
                            let challenge_claim = match kind {
                                RankedAdmissionKind::Fresh => session.prepare_join(
                                    u16::from(seat),
                                    PublicKey32::from_bytes(durable_public_key),
                                    PublicKey32::from_bytes(identity.transport_endpoint_id),
                                    PublicKey32::from_bytes(*context.host_endpoint_id.as_bytes()),
                                ),
                                RankedAdmissionKind::Reconnect => session.prepare_reconnect(
                                    u16::from(seat),
                                    PublicKey32::from_bytes(durable_public_key),
                                    PublicKey32::from_bytes(identity.transport_endpoint_id),
                                    PublicKey32::from_bytes(*context.host_endpoint_id.as_bytes()),
                                ),
                            };
                            match challenge_claim {
                                Ok(claim) => {
                                    let documents = encode_ranked_wire_document(session.genesis())
                                        .map_err(|error| error.to_string())
                                        .and_then(|bytes| {
                                            RankedSessionGenesisDocument::new(bytes)
                                                .map_err(str::to_string)
                                        })
                                        .and_then(|genesis| {
                                            encode_ranked_wire_document(&claim)
                                                .map_err(|error| error.to_string())
                                                .and_then(|bytes| {
                                                    RankedJoinClaimDocument::new(bytes)
                                                        .map_err(str::to_string)
                                                })
                                                .map(|join_claim| RankedJoinChallenge {
                                                    session_genesis: genesis,
                                                    join_claim,
                                                })
                                        });
                                    match documents {
                                        Ok(challenge) => {
                                            let generation =
                                                *peers.session_generations.get(&seat).expect(
                                                    "provisional seat has a session generation",
                                                );
                                            let sender = peers
                                                .senders
                                                .get(&seat)
                                                .cloned()
                                                .expect("provisional seat has a sender");
                                            peers.pending_ranked_admission =
                                                Some(PendingRankedAdmission {
                                                    seat,
                                                    generation,
                                                    kind,
                                                    challenge: challenge.clone(),
                                                    deadline: Instant::now()
                                                        + RANKED_ADMISSION_TIMEOUT,
                                                });
                                            RankedAdmissionProgress::Challenge { sender, challenge }
                                        }
                                        Err(detail) => {
                                            session.cancel_pending_join();
                                            RankedAdmissionProgress::Downgrade {
                                                reason:
                                                    RankedBrowseOnlyReason::RankedProtocolViolation,
                                                detail: format!(
                                                    "could not encode ranked admission challenge: {detail}"
                                                ),
                                            }
                                        }
                                    }
                                }
                                Err(error) => RankedAdmissionProgress::Downgrade {
                                    reason: RankedBrowseOnlyReason::PeerAttestationRejected,
                                    detail: format!(
                                        "could not prepare ranked admission for seat {seat}: {error}"
                                    ),
                                },
                            }
                        } else {
                            RankedAdmissionProgress::Downgrade {
                                reason: RankedBrowseOnlyReason::PeerIdentityUnavailable,
                                detail: format!("seat {seat} has no durable ranked identity"),
                            }
                        }
                    }
                }
            }
        };
        match progress {
            RankedAdmissionProgress::Idle => return,
            RankedAdmissionProgress::BrowseOnly => {
                connect_all_provisional_seats(context);
                return;
            }
            RankedAdmissionProgress::Challenge { sender, challenge } => {
                if sender.send(NetMsg::RankedJoinChallenge(challenge)).is_err() {
                    downgrade_ranked_session(
                        context,
                        RankedBrowseOnlyReason::RankedTransportInterrupted,
                        "ranked admission target disconnected before challenge delivery",
                    );
                }
                return;
            }
            RankedAdmissionProgress::Downgrade { reason, detail } => {
                downgrade_ranked_session(context, reason, detail);
                return;
            }
        }
    }
}

fn resolve_ranked_before_ready(context: &ServerContext) {
    let awaiting = ranked_lifecycle_lock(&context.ranked_lifecycle).is_awaiting_prepared_inputs();
    if awaiting {
        downgrade_ranked_session(
            context,
            RankedBrowseOnlyReason::HostRankedSessionUnavailable,
            "authoritative simulation became ready before ranked prepared inputs were installed",
        );
    } else {
        progress_ranked_admission(context);
    }
}

fn ranked_unavailable_browse_reason(
    reason: super::RankedJoinUnavailableReason,
) -> RankedBrowseOnlyReason {
    match reason {
        super::RankedJoinUnavailableReason::DurableIdentityUnavailable => {
            RankedBrowseOnlyReason::PeerIdentityUnavailable
        }
        super::RankedJoinUnavailableReason::LocalRankedSessionUnavailable
        | super::RankedJoinUnavailableReason::LocalRankedSessionMismatch => {
            RankedBrowseOnlyReason::PeerRankedSessionMismatch
        }
        super::RankedJoinUnavailableReason::AttestationSigningFailed => {
            RankedBrowseOnlyReason::PeerAttestationRejected
        }
    }
}

fn handle_ranked_join_response(
    context: &ServerContext,
    seat: PlayerId,
    generation: u64,
    identity: RankedPeerIdentity,
    response: RankedJoinResponse,
) {
    if context.peers.lock().session_generations.get(&seat.0) != Some(&generation) {
        tracing::debug!(
            ?seat,
            generation,
            "ignoring ranked response from superseded stream"
        );
        return;
    }
    let RankedJoinResponse::Attestation(attestation_document) = response else {
        let RankedJoinResponse::Unavailable(reason) = response else {
            unreachable!("ranked join response has only closed variants")
        };
        downgrade_ranked_session(
            context,
            ranked_unavailable_browse_reason(reason),
            format!(
                "seat {} reported ranked admission unavailable: {reason:?}",
                seat.0
            ),
        );
        return;
    };
    let attestation = match decode_ranked_wire_document(attestation_document.as_bytes()) {
        Ok(attestation) => attestation,
        Err(error) => {
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::PeerAttestationRejected,
                format!(
                    "seat {} sent an invalid ranked attestation: {error}",
                    seat.0
                ),
            );
            return;
        }
    };

    let admitted = {
        let mut lifecycle = ranked_lifecycle_lock(&context.ranked_lifecycle);
        if lifecycle.browse_only_reason().is_some() {
            return;
        }
        let Some(session) = lifecycle.ranked_mut() else {
            drop(lifecycle);
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::RankedProtocolViolation,
                "ranked attestation arrived before host prepared inputs were installed",
            );
            return;
        };
        let mut peers = context.peers.lock();
        let Some(pending) = peers.pending_ranked_admission.clone() else {
            drop(peers);
            drop(lifecycle);
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::RankedProtocolViolation,
                format!(
                    "seat {} sent a ranked attestation without a pending challenge",
                    seat.0
                ),
            );
            return;
        };
        if pending.seat != seat.0 || pending.generation != generation {
            drop(peers);
            drop(lifecycle);
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::RankedProtocolViolation,
                format!("seat {} answered another seat's ranked challenge", seat.0),
            );
            return;
        }
        let result = match pending.kind {
            RankedAdmissionKind::Fresh => session.admit_join(
                attestation,
                identity.transport_endpoint_id,
                identity.public_disclosure,
            ),
            RankedAdmissionKind::Reconnect => {
                session.admit_reconnect(attestation, identity.transport_endpoint_id)
            }
        };
        match result {
            Ok(()) => {
                let roster = session.participant_claims();
                let roster_document = encode_ranked_wire_document(&roster)
                    .map_err(|error| format!("encode ranked participant roster: {error}"))
                    .and_then(|bytes| {
                        RankedParticipantRosterDocument::new(bytes)
                            .map_err(|error| format!("encode ranked participant roster: {error}"))
                    });
                match roster_document {
                    Err(error) => Err(error),
                    Ok(roster_document) => {
                        let roster_targets = if pending.kind == RankedAdmissionKind::Fresh {
                            peers
                                .sim_connected_seats
                                .iter()
                                .filter_map(|existing_seat| {
                                    peers.senders.get(existing_seat).cloned()
                                })
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };
                        peers.pending_ranked_admission = None;
                        assert!(
                            peers.sim_connected_seats.insert(seat.0),
                            "ranked admission connected a seat already present in the simulation"
                        );
                        let nickname = peers.nicknames.get(&seat.0).cloned().unwrap_or_else(|| {
                            panic!("admitted ranked seat {} has no nickname", seat.0)
                        });
                        let sender = peers.senders.get(&seat.0).cloned().unwrap_or_else(|| {
                            panic!("admitted ranked seat {} has no sender", seat.0)
                        });
                        Ok((pending, nickname, sender, roster_document, roster_targets))
                    }
                }
            }
            Err(error) => Err(error.to_string()),
        }
    };

    let (pending, nickname, sender, roster_document, roster_targets) = match admitted {
        Ok(admitted) => admitted,
        Err(error) => {
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::PeerAttestationRejected,
                format!("seat {} ranked attestation was rejected: {error}", seat.0),
            );
            return;
        }
    };
    let accepted = RankedJoinAccepted {
        session_genesis: pending.challenge.session_genesis,
        join_attestation: attestation_document,
        participant_roster: roster_document.clone(),
    };
    if sender.send(NetMsg::RankedJoinAccepted(accepted)).is_err() {
        downgrade_ranked_session(
            context,
            RankedBrowseOnlyReason::RankedTransportInterrupted,
            format!(
                "seat {} disconnected before ranked acknowledgement delivery",
                seat.0
            ),
        );
        return;
    }
    for roster_target in roster_targets {
        if roster_target
            .send(NetMsg::RankedParticipantRoster(roster_document.clone()))
            .is_err()
        {
            downgrade_ranked_session(
                context,
                RankedBrowseOnlyReason::RankedTransportInterrupted,
                "an admitted participant disconnected before ranked roster update delivery",
            );
            return;
        }
    }
    publish_connect_seats(context, vec![(seat.0, nickname)]);
    finish_ranked_seat_connections(context, &[seat.0]);
    progress_ranked_admission(context);
}

async fn run_server_accept_loop(context: Arc<ServerContext>, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let context = Arc::clone(&context);
        tokio::spawn(async move {
            let cancelled = context.cancellation.load(Ordering::Acquire);
            if let Err(e) = handle_incoming_peer(context.clone(), incoming).await
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
    context: Arc<ServerContext>,
    incoming: iroh::endpoint::Incoming,
) -> Result<(), String> {
    let conn = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, incoming)
        .await
        .map_err(|_| "peer QUIC handshake timed out".to_owned())?
        .map_err(|e| format!("peer connecting: {e}"))?;
    let remote_id = conn.remote_id();
    let peer_id = remote_id.to_string();
    tracing::info!(peer = %peer_id, "incoming connection");

    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| "peer did not open a game stream before handshake timeout".to_owned())?
        .map_err(|e| format!("accept peer stream: {e}"))?;

    // Receive Hello.  Reject anything else.
    let (nickname, browser_auth, ranked_public_key) = match read_frame_bounded_with_timeout(
        &mut recv,
        InboundFramePolicy::ClientHello,
        HANDSHAKE_FRAME_TIMEOUT,
        "client Hello",
    )
    .await?
    {
        Some(NetMsg::Hello {
            protocol_version,
            nickname,
            browser_auth,
            ranked_public_key,
        }) => {
            if protocol_version != NET_PROTOCOL_VERSION {
                let reason = format!(
                    "protocol mismatch (peer={protocol_version}, server={NET_PROTOCOL_VERSION})"
                );
                reject_opening(&mut send, &reason).await;
                return Err(reason);
            }
            (nickname, browser_auth, ranked_public_key)
        }
        Some(other) => {
            let reason = format!("expected Hello, got {other:?}");
            reject_opening(&mut send, &reason).await;
            return Err(reason);
        }
        None => return Err("connection closed before Hello".to_string()),
    };

    let owner = match authenticate_peer(&context, remote_id, browser_auth.as_ref()) {
        Ok(owner) => owner,
        Err(reason) => {
            reject_opening(&mut send, &reason).await;
            return Err(reason);
        }
    };
    let durable_public_key = match (owner, ranked_public_key) {
        (PeerOwner::Browser(owner_key), Some(ranked_key)) if owner_key != ranked_key => {
            tracing::warn!(peer = %peer_id, "browser gameplay owner and ranked durable key differ");
            None
        }
        (_, ranked_public_key) => ranked_public_key,
    };
    let ranked_identity = RankedPeerIdentity {
        durable_public_key,
        transport_endpoint_id: *remote_id.as_bytes(),
        public_disclosure: ParticipantPublicDisclosureV1::NamedProfile,
    };
    if let Some(content) = &context.content {
        let joining = admit_distributed_mod(
            &mut send,
            &mut recv,
            content,
            context.host_endpoint_id.to_string(),
        )
        .await?;
        if !joining {
            conn.close(CLOSE_GRACEFUL.into(), b"content prepared");
            return Ok(());
        }
    }

    // Claim/reclaim a seat by authenticated owner, never by editable nickname.
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
        p.claim_seat(owner, &nickname, ranked_identity, write_tx)
            .map(|claim| (claim, write_rx))
    };
    let (seat_claim, mut write_rx) = match seat_claim {
        Ok(claim) => claim,
        Err(reason) => {
            reject_opening(&mut send, &reason).await;
            return Err(reason);
        }
    };
    let assigned_seat_u8 = seat_claim.seat;
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(assigned_seat_u8);
    let ranked_admission_required = ranked_lifecycle_lock(&context.ranked_lifecycle)
        .ranked_session()
        .is_some();

    // Queue Welcome for this peer.  Goes through the writer queue so
    // the writer task is the only thing that touches the outbound
    // half of the stream.  If the host has cached an initial-state
    // snapshot we follow up with that — mid-mission joiners adopt it
    // instead of trying to reproduce engine init from seed alone.
    let opening_result = (|| -> Result<(), String> {
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
            if let Some(transition) = p.snapshot_transition.as_ref() {
                sender
                    .send(NetMsg::PrepareSnapshotTransition {
                        id: transition.id,
                        payload: transition.payload.clone(),
                    })
                    .map_err(|_| "writer queue closed before transition Prepare")?;
            }
            if let Some(reason) = *context.ranked_browse_reason.lock() {
                sender
                    .send(NetMsg::RankedBrowseOnly { reason })
                    .map_err(|_| "writer queue closed before ranked browse-only status")?;
            }
            // An admitted ranked participant must re-attest this replacement
            // transport before a cached gameplay release is replayed. The
            // ranked acceptance path sends the cached BeginSim afterward.
            if !ranked_admission_required && let Some((frame, start_epoch_ms)) = p.begin_sent {
                let begin_frame =
                    snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
                let begin_start_epoch_ms = if begin_frame != frame {
                    current_epoch_ms()?.checked_add(100).ok_or_else(|| {
                        "multiplayer reconnect timestamp exceeds the u64 Unix range".to_owned()
                    })?
                } else {
                    start_epoch_ms
                };
                sender
                    .send(NetMsg::BeginSim {
                        frame: begin_frame,
                        start_epoch_ms: begin_start_epoch_ms,
                    })
                    .map_err(|_| "writer queue closed before cached BeginSim")?;
            }
        }
        Ok(())
    })();
    if let Err(error) = opening_result {
        let mut p = context.peers.lock();
        p.release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        return Err(error);
    }

    if seat_claim.kind == SeatClaimKind::ActiveReplacement {
        let changes_ranked_transport = ranked_lifecycle_lock(&context.ranked_lifecycle)
            .ranked_session()
            .and_then(|session| {
                session
                    .participant_claims()
                    .into_iter()
                    .find(|participant| participant.seat == u16::from(assigned_seat_u8))
            })
            .and_then(|participant| participant.join_attestation)
            .is_some_and(|join| {
                *join.claim.transport_endpoint_id.as_bytes()
                    != ranked_identity.transport_endpoint_id
            });
        if changes_ranked_transport {
            downgrade_ranked_session(
                &context,
                RankedBrowseOnlyReason::RankedTransportInterrupted,
                format!(
                    "active replacement for seat {assigned_seat_u8} changed its authenticated ranked transport"
                ),
            );
        }
    }

    // The stream is gameplay-compatible after Welcome, but the deterministic
    // seat does not enter the replay until ranked admission succeeds or the
    // whole session irreversibly downgrades to browse-only.
    progress_ranked_admission(&context);
    let admission_monitor = tokio::spawn(monitor_ranked_admission(
        Arc::clone(&context),
        assigned_seat_u8,
        session_generation,
    ));

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
        let reader = run_server_peer_reader(
            &context,
            assigned_seat,
            session_generation,
            ranked_identity,
            &mut recv,
        );
        tokio::select! {
            result = reader => result,
            result = writer => result.map_err(|e| format!("peer writer: {e}")),
        }
    };

    // On disconnect, park the authenticated owner identity so a future
    // reconnect reclaims the same deterministic seat. Nicknames are labels.
    let release = {
        let mut p = context.peers.lock();
        let release = p.release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        if release.is_some() {
            retain_transition_peer_for_reconnect(&mut p, assigned_seat_u8);
        }
        release
    };
    if release == Some(true) && !context.cancellation.load(Ordering::Acquire) {
        let observation = {
            let mut lifecycle = ranked_lifecycle_lock(&context.ranked_lifecycle);
            lifecycle
                .ranked_mut()
                .map(|session| session.observe_disconnect(u16::from(assigned_seat_u8)))
        };
        if let Some(Err(error)) = observation {
            downgrade_ranked_session(
                &context,
                RankedBrowseOnlyReason::RankedProtocolViolation,
                format!("could not record ranked disconnect for seat {assigned_seat_u8}: {error}"),
            );
        }
        let now = context.frame_cursor.load(Ordering::Relaxed);
        let target = now.saturating_add(INPUT_DELAY_FRAMES);
        let inp = PlayerInput::new(
            PlayerId::HOST,
            PlayerCommand::DisconnectSeat {
                player_id: assigned_seat,
            },
        );
        broadcast_input(&context, now, now, target, inp);
    } else if release == Some(false) {
        let cancelled_pending = {
            let mut peers = context.peers.lock();
            let matches = peers
                .pending_ranked_admission
                .as_ref()
                .is_some_and(|pending| {
                    pending.seat == assigned_seat_u8 && pending.generation == session_generation
                });
            if matches {
                peers.pending_ranked_admission = None;
            }
            matches
        };
        if cancelled_pending {
            if let Some(session) = ranked_lifecycle_lock(&context.ranked_lifecycle).ranked_mut() {
                session.cancel_pending_join();
            }
        }
    }
    progress_ranked_admission(&context);
    conn.close(CLOSE_GRACEFUL.into(), b"session over");
    admission_monitor.abort();

    result
}

async fn monitor_ranked_admission(context: Arc<ServerContext>, seat: u8, generation: u64) {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let status = {
            let peers = context.peers.lock();
            if peers.session_generations.get(&seat) != Some(&generation)
                || peers.sim_connected_seats.contains(&seat)
            {
                0
            } else {
                peers
                    .pending_ranked_admission
                    .as_ref()
                    .filter(|pending| pending.seat == seat && pending.generation == generation)
                    .map_or(1, |pending| {
                        if Instant::now() >= pending.deadline {
                            2
                        } else {
                            1
                        }
                    })
            }
        };
        match status {
            0 => return,
            1 => {}
            2 => {
                downgrade_ranked_session(
                    &context,
                    RankedBrowseOnlyReason::RankedTransportInterrupted,
                    format!("seat {seat} timed out during ranked admission"),
                );
                return;
            }
            _ => unreachable!(),
        }
    }
}

async fn admit_distributed_mod(
    send: &mut SendStream,
    recv: &mut RecvStream,
    content: &HostedModContent,
    host_endpoint_id: String,
) -> Result<bool, String> {
    let offer = content.offer(host_endpoint_id)?;
    write_frame_with_timeout(
        send,
        &NetMsg::ContentOffer {
            offer: offer.clone(),
        },
        CONTENT_TRANSFER_IDLE_TIMEOUT,
        "content offer",
    )
    .await?;
    let resume_offset = match read_frame_bounded_with_timeout(
        recv,
        InboundFramePolicy::ClientToServer,
        CONTENT_DECISION_TIMEOUT,
        "content decision",
    )
    .await?
    {
        Some(NetMsg::ContentRequest {
            full_mod_sha256,
            resume_offset,
        }) if full_mod_sha256 == offer.full_mod_sha256 => resume_offset,
        Some(NetMsg::ContentRequest {
            full_mod_sha256, ..
        }) => {
            return Err(format!(
                "client requested distributed mod {}, offered {}",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
            ));
        }
        Some(NetMsg::ContentReject {
            full_mod_sha256,
            reason,
        }) if full_mod_sha256 == offer.full_mod_sha256 => {
            return Err(format!("client declined exact host content: {reason}"));
        }
        Some(other) => return Err(format!("expected ContentRequest, got {other:?}")),
        None => return Err("connection closed before content decision".to_owned()),
    };
    if resume_offset > content.encoded.len() as u64 {
        return Err(format!(
            "client resume offset {resume_offset} exceeds content length {}",
            content.encoded.len()
        ));
    }
    let mut offset = resume_offset as usize;
    let transfer_deadline = tokio::time::Instant::now() + CONTENT_DECISION_TIMEOUT;
    while offset < content.encoded.len() {
        let end = (offset + robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT)
            .min(content.encoded.len());
        let message = NetMsg::ContentChunk {
            full_mod_sha256: offer.full_mod_sha256,
            offset: offset as u64,
            total_bytes: content.encoded.len() as u64,
            bytes: content.encoded[offset..end].to_vec(),
        };
        tokio::select! {
            result = write_frame_with_timeout(
                send,
                &message,
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content chunk",
            ) => result?,
            _ = tokio::time::sleep_until(transfer_deadline) => {
                return Err(format!("content transfer exceeded {CONTENT_DECISION_TIMEOUT:?}"));
            }
        }
        offset = end;
    }
    match read_frame_bounded_with_timeout(
        recv,
        InboundFramePolicy::ClientToServer,
        CONTENT_READINESS_TIMEOUT,
        "content readiness",
    )
    .await?
    {
        Some(NetMsg::ContentReady { full_mod_sha256 })
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            Ok(true)
        }
        Some(NetMsg::ContentPrepared { full_mod_sha256 })
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            Ok(false)
        }
        Some(NetMsg::ContentReject {
            full_mod_sha256,
            reason,
        }) if full_mod_sha256 == offer.full_mod_sha256 => {
            Err(format!("client rejected downloaded host content: {reason}"))
        }
        Some(other) => Err(format!(
            "expected ContentReady/ContentPrepared, got {other:?}"
        )),
        None => Err("connection closed before content readiness".to_owned()),
    }
}

async fn reject_opening(send: &mut SendStream, reason: &str) {
    let reason = robin_engine::multiplayer::bounded_safe_diagnostic(
        reason,
        robin_engine::multiplayer::MAX_REJECT_REASON_BYTES,
    );
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
    ticket.validate_use_at(current_epoch_ms()? / 1000, use_kind)?;
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
    session_generation: u64,
    ranked_identity: RankedPeerIdentity,
    recv: &mut RecvStream,
) -> Result<(), String> {
    loop {
        let Some(message) = read_frame(recv, InboundFramePolicy::ClientToServer).await? else {
            return Ok(());
        };
        validate_server_gameplay_wire_msg(&message)?;
        match message {
            NetMsg::Input {
                origin_frame,
                command,
            } => {
                validate_peer_command_authority(seat, &command)?;
                let now = context.frame_cursor.load(Ordering::Relaxed);
                let target = now.max(origin_frame).saturating_add(INPUT_DELAY_FRAMES);
                let inp = PlayerInput::new(seat, command);
                broadcast_input(context, now, origin_frame, target, inp);
            }
            NetMsg::Note(s) => {
                tracing::info!(?seat, note = %s, "peer note");
            }
            NetMsg::ModalProposal {
                instance,
                kind,
                result,
                requested_frame,
            } => {
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
            NetMsg::ModalDecision { .. } => {
                return Err(format!(
                    "peer {seat:?} attempted an authoritative modal decision"
                ));
            }
            NetMsg::ReadyToSim { frame } => {
                resolve_ranked_before_ready(context);
                let begin = {
                    let mut p = context.peers.lock();
                    p.ready_seats.insert(seat.0, frame);
                    maybe_begin_sim_locked(&mut p)
                };
                let begin = match begin {
                    Ok(begin) => begin,
                    Err(error) => {
                        fail_server(context, error.clone());
                        return Err(error);
                    }
                };
                announce_begin_sim(context, begin);
            }
            NetMsg::SnapshotTransitionReady { id } => {
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
            NetMsg::LeaderboardCoSignResponse(response) => {
                if ranked_lifecycle_lock(&context.ranked_lifecycle)
                    .ranked_session()
                    .is_none()
                {
                    tracing::warn!(
                        ?seat,
                        "ignored leaderboard co-sign response after ranked eligibility ended"
                    );
                    continue;
                }
                {
                    let mut peers = context.peers.lock();
                    peers.complete_leaderboard_cosign(seat, &response)?;
                }
                context
                    .incoming_tx
                    .send(NetEvent::LeaderboardCoSignResponse {
                        from: seat,
                        response,
                    })
                    .map_err(|_| {
                        "host leaderboard co-sign response channel is closed".to_string()
                    })?;
            }
            NetMsg::RankedContinuationReceiptSelection(selection) => {
                let decoded = decode_ranked_wire_document::<
                    CampaignContinuationReceiptSelectionResponseV1,
                >(selection.as_bytes())
                .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
                let expected_key = context
                    .peers
                    .lock()
                    .ranked_identities
                    .get(&seat.0)
                    .and_then(|identity| identity.durable_public_key)
                    .map(PublicKey32::from_bytes)
                    .ok_or_else(|| {
                        format!(
                            "peer {seat:?} has no authenticated durable identity for continuation receipt selection"
                        )
                    })?;
                if decoded.responder_public_key() != expected_key {
                    return Err(format!(
                        "peer {seat:?} selected a campaign receipt controlled by another durable identity"
                    ));
                }
                context
                    .incoming_tx
                    .send(NetEvent::RankedContinuationReceiptSelection {
                        from: seat,
                        selection,
                    })
                    .map_err(|_| {
                        "host continuation receipt selection channel is closed".to_string()
                    })?;
            }
            NetMsg::RankedContinuationPreflightSignature(signature) => {
                let signature_document =
                    crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                        ParticipantSignatureV1,
                    >(signature.as_bytes())
                    .map_err(|error| {
                        format!("invalid continuation preflight signature: {error}")
                    })?;
                if signature_document.public_key.is_zero() || signature_document.signature.is_zero()
                {
                    return Err(
                        "continuation preflight signature contains zero key material".to_string(),
                    );
                }
                let expected_key = context
                    .peers
                    .lock()
                    .ranked_identities
                    .get(&seat.0)
                    .and_then(|identity| identity.durable_public_key)
                    .map(PublicKey32::from_bytes)
                    .ok_or_else(|| {
                        format!(
                            "peer {seat:?} has no authenticated durable identity for continuation preflight"
                        )
                    })?;
                if signature_document.public_key != expected_key {
                    return Err(format!(
                        "peer {seat:?} signed continuation preflight with a key other than its authenticated durable identity"
                    ));
                }
                context
                    .incoming_tx
                    .send(NetEvent::RankedContinuationPreflightSignature {
                        from: seat,
                        signature,
                    })
                    .map_err(|_| {
                        "host continuation preflight signature channel is closed".to_string()
                    })?;
            }
            NetMsg::RankedJoinResponse(response) => {
                handle_ranked_join_response(
                    context,
                    seat,
                    session_generation,
                    ranked_identity,
                    response,
                );
            }
            NetMsg::RankedJoinChallenge(_)
            | NetMsg::RankedJoinAccepted(_)
            | NetMsg::RankedParticipantRoster(_)
            | NetMsg::RankedBrowseOnly { .. }
            | NetMsg::RankedCoSignContext(_)
            | NetMsg::RankedSubmissionAccepted(_)
            | NetMsg::RankedOfficialSessionSetup(_)
            | NetMsg::RankedContinuationReceiptSelectionRequest(_)
            | NetMsg::RankedContinuationPreflightClaim(_) => {
                return Err(format!(
                    "peer {seat:?} attempted a server-only ranked control message"
                ));
            }
            NetMsg::LeaderboardCoSignRequest(_) => {
                return Err(format!(
                    "peer {seat:?} attempted a server-only leaderboard co-sign request"
                ));
            }
            NetMsg::PrepareSnapshotTransition { .. } | NetMsg::CommitSnapshotTransition { .. } => {
                return Err(format!(
                    "peer {seat:?} attempted a host-only snapshot transition message"
                ));
            }
            _ => unreachable!("server gameplay message was validated before dispatch"),
        }
    }
}

fn validate_server_gameplay_wire_msg(message: &NetMsg) -> Result<(), String> {
    match message {
        NetMsg::Input { .. }
        | NetMsg::Note(_)
        | NetMsg::ModalProposal { .. }
        | NetMsg::ReadyToSim { .. }
        | NetMsg::SnapshotTransitionReady { .. }
        | NetMsg::LeaderboardCoSignResponse(_)
        | NetMsg::RankedContinuationReceiptSelection(_)
        | NetMsg::RankedContinuationPreflightSignature(_)
        | NetMsg::RankedJoinResponse(_) => Ok(()),
        NetMsg::ContentRequest { .. }
        | NetMsg::ContentReject { .. }
        | NetMsg::ContentReady { .. }
        | NetMsg::ContentPrepared { .. } => {
            Err("content-admission message arrived in an ordinary peer session".to_owned())
        }
        other => Err(format!(
            "client sent invalid server-session message {other:?}"
        )),
    }
}

fn validate_peer_command_authority(
    seat: PlayerId,
    command: &robin_engine::player_command::PlayerCommand,
) -> Result<(), String> {
    if command.requires_host_authority() {
        return Err(format!(
            "peer {seat:?} attempted host-authoritative command {command:?}"
        ));
    }
    Ok(())
}

// ─── Client ──────────────────────────────────────────────────────

/// Handle to an active client connection.
pub struct ClientHandle {
    /// Seat assigned by the server.  `None` until the handshake
    /// completes.  Game loop reads this to set `host.local_seat`.
    pub assigned_seat: Arc<Mutex<Option<PlayerId>>>,
    session_id: Arc<Mutex<Option<MultiplayerSessionId>>>,
    /// Mission RNG seed announced by the server in `Welcome`.  The
    /// client adopts this seed for its engine init so the local sim
    /// rolls match the host's.
    pub mission_seed: Arc<Mutex<Option<u64>>>,
    pub mission_sim_config: Arc<Mutex<Option<robin_engine::engine::SimConfig>>>,
    pub mission_id: Arc<Mutex<Option<String>>>,
    /// Outer `None` means Welcome is still pending; `Some(None)` is the
    /// host's authoritative choice of base `Data/Sounds` timing.
    pub speech_timing_locale: Arc<Mutex<Option<Option<String>>>>,
    /// Present when the host requires content admission before Welcome. The
    /// game/menu must explicitly trust, download, validate, mount, and answer
    /// this exact offer; the transport never silently approves it.
    pub content_offer: Arc<Mutex<Option<robin_engine::multiplayer::DistributedModOffer>>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_setup_tx:
        UnboundedSender<Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>>,
    ranked_setup_sent: AtomicBool,
    ranked_local_public_key: Option<PublicKey32>,
    ranked_authenticated_host_public_key: PublicKey32,
    cancellation: Arc<AtomicBool>,
    io_thread: Option<JoinHandle<()>>,
}

impl ClientHandle {
    pub fn session_id(&self) -> Option<MultiplayerSessionId> {
        *self.session_id.lock()
    }

    pub(crate) fn ranked_lifecycle(&self) -> SharedRankedSessionLifecycle {
        Arc::clone(&self.ranked_lifecycle)
    }

    pub(crate) fn ranked_local_seat(&self) -> Result<PlayerId, String> {
        self.assigned_seat()
            .ok_or_else(|| "ranked client seat is unavailable before handshake".to_string())
    }

    pub(crate) fn ranked_local_public_key(&self) -> Option<PublicKey32> {
        self.ranked_local_public_key
    }

    pub(crate) fn ranked_authenticated_host_public_key(&self) -> Option<PublicKey32> {
        Some(self.ranked_authenticated_host_public_key)
    }

    pub(crate) fn install_ranked_session_setup(
        &self,
        setup: Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    ) -> Result<(), String> {
        if let Some(setup) = setup.as_ref() {
            setup
                .validate()
                .map_err(|error| format!("invalid official ranked client setup: {error}"))?;
        }
        if self.ranked_setup_sent.swap(true, Ordering::AcqRel) {
            return Err("ranked client setup was already resolved".to_string());
        }
        self.ranked_setup_tx
            .send(setup)
            .map_err(|_| "ranked client setup channel is closed".to_string())
    }

    pub fn assigned_seat(&self) -> Option<PlayerId> {
        *self.assigned_seat.lock()
    }

    pub fn mission_seed(&self) -> Option<u64> {
        *self.mission_seed.lock()
    }

    pub fn mission_sim_config(&self) -> Option<robin_engine::engine::SimConfig> {
        *self.mission_sim_config.lock()
    }

    pub fn mission_id(&self) -> Option<String> {
        self.mission_id.lock().clone()
    }

    pub fn speech_timing_locale(&self) -> Option<String> {
        self.speech_timing_locale.lock().clone().flatten()
    }

    /// The outer option distinguishes a pending handshake from an explicit
    /// `None`, which authoritatively selects base `Data/Sounds` timing.
    pub fn speech_timing_authority(&self) -> Option<Option<String>> {
        self.speech_timing_locale.lock().clone()
    }

    pub fn content_offer(&self) -> Option<robin_engine::multiplayer::DistributedModOffer> {
        self.content_offer.lock().clone()
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
/// This standalone entry point binds a fresh ephemeral iroh identity and keeps
/// the install's durable game identity separate for ranked attestation. A
/// durable-key storage failure never prevents otherwise-compatible gameplay;
/// that client joins without ranked authority and the session is downgraded by
/// the authenticated admission protocol.
pub fn connect_client(
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_in_campaign(
        &MultiplayerCampaignSession::default(),
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

pub fn connect_client_in_campaign(
    campaign: &MultiplayerCampaignSession,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    let durable_ranked_key = match game_secret_key() {
        Ok(key) => Some(key),
        Err(error) => {
            tracing::warn!(%error, "durable ranked identity unavailable; multiplayer gameplay will remain available browse-only");
            None
        }
    };
    connect_client_inner(
        campaign.state().client_key.clone(),
        durable_ranked_key,
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

/// Explicit transport/ranking identity seam for real-iroh tests. Keeping the
/// two keys independently injectable prevents tests from accidentally blessing
/// the transport endpoint as durable ranking authority.
#[cfg(test)]
pub(crate) fn connect_client_with_keys(
    transport_key: SecretKey,
    durable_ranked_key: Option<SecretKey>,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(
        transport_key,
        durable_ranked_key,
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

/// Explicit-key client entry used by transport tests and isolated tooling.
/// The injected key owns both the test transport and its ranked identity;
/// production still derives only the durable ranked key from install state.
pub fn connect_client_with_key(
    key: SecretKey,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    connect_client_inner(
        key.clone(),
        Some(key),
        addr,
        nickname,
        incoming_tx,
        outgoing_rx,
    )
}

fn connect_client_inner(
    transport_key: SecretKey,
    durable_ranked_key: Option<SecretKey>,
    addr: impl AsRef<str>,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
) -> std::io::Result<ClientHandle> {
    robin_engine::multiplayer::validate_display_name(&nickname).map_err(std::io::Error::other)?;
    let server_addr = parse_connect_addr(addr.as_ref()).map_err(std::io::Error::other)?;
    let ranked_authenticated_host_public_key = PublicKey32::from_bytes(*server_addr.id.as_bytes());
    let ranked_local_public_key = durable_ranked_key
        .as_ref()
        .map(|key| PublicKey32::from_bytes(*key.public().as_bytes()));
    let addr_display = addr.as_ref().to_string();
    let assigned_seat = Arc::new(Mutex::new(None));
    let ranked_lifecycle = Arc::new(std::sync::Mutex::new(
        RankedSessionLifecycle::awaiting_prepared_inputs(),
    ));
    let ranked_lifecycle_for_thread = Arc::clone(&ranked_lifecycle);
    let (ranked_setup_tx, mut ranked_setup_rx) = unbounded_channel();
    let assigned_clone = Arc::clone(&assigned_seat);
    let session_id = Arc::new(Mutex::new(None));
    let session_id_for_thread = Arc::clone(&session_id);
    let mission_seed = Arc::new(Mutex::new(None));
    let mission_seed_for_thread = Arc::clone(&mission_seed);
    let mission_sim_config = Arc::new(Mutex::new(None));
    let mission_sim_config_for_thread = Arc::clone(&mission_sim_config);
    let mission_id = Arc::new(Mutex::new(None));
    let mission_id_for_thread = Arc::clone(&mission_id);
    let speech_timing_locale = Arc::new(Mutex::new(None));
    let speech_timing_locale_for_thread = Arc::clone(&speech_timing_locale);
    let content_offer = Arc::new(Mutex::new(None));
    let content_offer_for_thread = Arc::clone(&content_offer);
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
                    transport_key,
                    durable_ranked_key,
                    server_addr,
                    nickname,
                    incoming_tx,
                    &mut outgoing_async_rx,
                    assigned_clone,
                    ranked_lifecycle_for_thread,
                    &mut ranked_setup_rx,
                    session_id_for_thread,
                    mission_id_for_thread,
                    mission_seed_for_thread,
                    mission_sim_config_for_thread,
                    speech_timing_locale_for_thread,
                    content_offer_for_thread,
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

    let initial = match handshake_rx.recv() {
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
    match initial {
        InitialHandshake::Welcomed { seat, mission_seed } => tracing::info!(
            addr = %addr_display,
            ?seat,
            seed = mission_seed,
            "multiplayer client connected"
        ),
        InitialHandshake::ContentOffered { full_mod_sha256 } => tracing::info!(
            addr = %addr_display,
            full_mod_sha256 = %robin_engine::spellforge::hex_hash(&full_mod_sha256),
            "multiplayer client awaiting exact host-content admission"
        ),
    }

    Ok(ClientHandle {
        assigned_seat,
        session_id,
        mission_seed,
        mission_sim_config,
        mission_id,
        speech_timing_locale,
        ranked_lifecycle,
        ranked_setup_tx,
        ranked_setup_sent: AtomicBool::new(false),
        ranked_local_public_key,
        ranked_authenticated_host_public_key,
        content_offer,
        cancellation,
        io_thread: Some(io_thread),
    })
}

/// A live client session: the connection plus its single
/// bidirectional message stream.
#[derive(Debug)]
struct ClientSession {
    // Held so the QUIC connection stays open for the streams' lifetime.
    _conn: Connection,
    send: SendStream,
    recv: RecvStream,
    protocol: super::client_protocol::ClientHandshake,
}

#[derive(Debug)]
enum HandshakePrelude {
    Welcome {
        session: ClientSession,
        welcome: WelcomeData,
    },
    Content {
        session: ClientSession,
        offer: robin_engine::multiplayer::DistributedModOffer,
    },
}

#[derive(Debug)]
enum InitialHandshake {
    Welcomed { seat: PlayerId, mission_seed: u64 },
    ContentOffered { full_mod_sha256: [u8; 32] },
}

/// One round of (connect → open stream → Hello → Welcome).  Used both
/// for the initial handshake and for the auto-retry path after
/// disconnects.
async fn handshake_async(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
    ranked_public_key: Option<[u8; 32]>,
) -> Result<HandshakePrelude, String> {
    let conn = endpoint
        .connect(server_addr.clone(), GAME_ALPN)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| format!("open stream: {e}"))?;

    write_frame_with_timeout(
        &mut send,
        &NetMsg::Hello {
            protocol_version: NET_PROTOCOL_VERSION,
            nickname: nickname.to_string(),
            browser_auth: None,
            ranked_public_key,
        },
        HANDSHAKE_FRAME_TIMEOUT,
        "client Hello",
    )
    .await
    .map_err(|e| format!("send Hello: {e}"))?;

    let message = read_frame_bounded_with_timeout(
        &mut recv,
        InboundFramePolicy::ServerToClient,
        HANDSHAKE_FRAME_TIMEOUT,
        "Welcome/content offer",
    )
    .await?;
    let mut protocol =
        super::client_protocol::ClientHandshake::new(server_addr.id.to_string(), None);
    let action = protocol.receive(message)?;
    let session = ClientSession {
        _conn: conn,
        send,
        recv,
        protocol,
    };
    match action {
        super::client_protocol::HandshakeAction::Welcome(welcome) => {
            Ok(HandshakePrelude::Welcome { session, welcome })
        }
        super::client_protocol::HandshakeAction::PrepareContent(offer) => {
            Ok(HandshakePrelude::Content { session, offer })
        }
    }
}

async fn handshake_or_cancel(
    endpoint: &Endpoint,
    server_addr: &EndpointAddr,
    nickname: &str,
    ranked_public_key: Option<[u8; 32]>,
    cancellation: &AtomicBool,
) -> Option<Result<HandshakePrelude, String>> {
    tokio::select! {
        result = handshake_async(endpoint, server_addr, nickname, ranked_public_key) => Some(result),
        _ = tokio::time::sleep(HANDSHAKE_FRAME_TIMEOUT) => {
            Some(Err(format!("multiplayer handshake timed out after {HANDSHAKE_FRAME_TIMEOUT:?}")))
        }
        _ = wait_for_cancel(cancellation) => None,
    }
}

async fn read_welcome(mut session: ClientSession) -> Result<(ClientSession, WelcomeData), String> {
    session.protocol.content_ready()?;
    let message = read_frame_bounded_with_timeout(
        &mut session.recv,
        InboundFramePolicy::ServerToClient,
        HANDSHAKE_FRAME_TIMEOUT,
        "post-content Welcome",
    )
    .await?;
    match session.protocol.receive(message)? {
        super::client_protocol::HandshakeAction::Welcome(welcome) => Ok((session, welcome)),
        super::client_protocol::HandshakeAction::PrepareContent(_) => {
            unreachable!("post-content phase only accepts Welcome")
        }
    }
}

/// Complete first-use admission under game/menu control. The transport
/// validates every wire invariant and does not send `ContentReady` itself;
/// that acknowledgement must come from the consumer after durable staging,
/// full-package hash validation, and deterministic mount preparation.
enum ContentAdmissionCompletion {
    Join(ClientSession, WelcomeData),
    Prepared,
}

async fn complete_content_admission(
    mut session: ClientSession,
    offer: &robin_engine::multiplayer::DistributedModOffer,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut UnboundedReceiver<NetOutbound>,
    cancellation: &AtomicBool,
) -> Result<ContentAdmissionCompletion, String> {
    let decision = tokio::select! {
        decision = outgoing_rx.recv() => decision.ok_or_else(|| "content admission channel closed".to_owned())?,
        _ = wait_for_cancel(cancellation) => return Err("content admission cancelled".to_owned()),
    };
    let mut received = match decision {
        NetOutbound::ContentRequest {
            full_mod_sha256,
            resume_offset,
        } if full_mod_sha256 == offer.full_mod_sha256 && resume_offset <= offer.encoded_bytes => {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentRequest {
                    full_mod_sha256,
                    resume_offset,
                },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content request",
            )
            .await?;
            resume_offset
        }
        NetOutbound::ContentRequest {
            full_mod_sha256,
            resume_offset,
        } => {
            return Err(format!(
                "invalid content request for {} at offset {resume_offset}; offered {} with {} bytes",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                offer.encoded_bytes
            ));
        }
        NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } if full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentReject {
                    full_mod_sha256,
                    reason: reason.clone(),
                },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content rejection",
            )
            .await?;
            return Err(format!(
                "local player declined exact host content: {reason}"
            ));
        }
        other => {
            return Err(format!(
                "expected local ContentRequest/ContentReject, got {other:?}"
            ));
        }
    };

    let transfer_deadline = tokio::time::Instant::now() + CONTENT_DECISION_TIMEOUT;
    while received < offer.encoded_bytes {
        let message = tokio::select! {
            message = read_frame_bounded_with_timeout(
                &mut session.recv,
                InboundFramePolicy::ServerToClient,
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content chunk",
            ) => message?,
            _ = wait_for_cancel(cancellation) => return Err("content transfer cancelled".to_owned()),
            _ = tokio::time::sleep_until(transfer_deadline) => {
                return Err(format!("content transfer exceeded {CONTENT_DECISION_TIMEOUT:?}"));
            }
        };
        let Some(NetMsg::ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        }) = message
        else {
            return Err(format!(
                "expected sequential ContentChunk at offset {received}, got {message:?}"
            ));
        };
        if full_mod_sha256 != offer.full_mod_sha256
            || total_bytes != offer.encoded_bytes
            || offset != received
            || bytes.is_empty()
            || bytes.len() > robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT
        {
            return Err(format!(
                "invalid distributed-mod chunk: hash={} offset={offset} total={total_bytes} bytes={}; expected hash={} offset={received} total={} and 1..={} bytes",
                robin_engine::spellforge::hex_hash(&full_mod_sha256),
                bytes.len(),
                robin_engine::spellforge::hex_hash(&offer.full_mod_sha256),
                offer.encoded_bytes,
                robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT
            ));
        }
        let end = received
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "distributed-mod chunk offset overflow".to_owned())?;
        if end > offer.encoded_bytes {
            return Err(format!(
                "distributed-mod chunk ends at {end}, beyond offered {} bytes",
                offer.encoded_bytes
            ));
        }
        let _ = incoming_tx.send(NetEvent::ContentChunk {
            full_mod_sha256,
            offset,
            total_bytes,
            bytes,
        });
        received = end;
    }

    let ready = tokio::select! {
        ready = outgoing_rx.recv() => ready.ok_or_else(|| "content readiness channel closed".to_owned())?,
        _ = wait_for_cancel(cancellation) => return Err("content readiness cancelled".to_owned()),
    };
    match ready {
        NetOutbound::ContentReady { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentReady { full_mod_sha256 },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content readiness",
            )
            .await?;
            let (session, welcome) = read_welcome(session).await?;
            return Ok(ContentAdmissionCompletion::Join(session, welcome));
        }
        NetOutbound::ContentPrepared { full_mod_sha256 }
            if full_mod_sha256 == offer.full_mod_sha256 =>
        {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentPrepared { full_mod_sha256 },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content prepared acknowledgement",
            )
            .await?;
            return Ok(ContentAdmissionCompletion::Prepared);
        }
        NetOutbound::ContentReject {
            full_mod_sha256,
            reason,
        } if full_mod_sha256 == offer.full_mod_sha256 => {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentReject {
                    full_mod_sha256,
                    reason: reason.clone(),
                },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content rejection",
            )
            .await?;
            return Err(format!(
                "downloaded host content failed local admission: {reason}"
            ));
        }
        other => {
            return Err(format!(
                "expected local ContentReady/ContentPrepared/ContentReject, got {other:?}"
            ));
        }
    }
}

/// A reconnect may bypass byte transfer only for the identical offer that
/// this same live client session already admitted and mounted. Any content
/// change or content/no-content downgrade is a hard reconnect failure.
async fn resolve_reconnect_prelude(
    prelude: HandshakePrelude,
    admitted: Option<&robin_engine::multiplayer::DistributedModOffer>,
) -> Result<(ClientSession, WelcomeData), String> {
    let offered = match &prelude {
        HandshakePrelude::Welcome { .. } => None,
        HandshakePrelude::Content { offer, .. } => Some(offer),
    };
    super::client_protocol::validate_reconnect_content(offered, admitted)?;
    match prelude {
        HandshakePrelude::Welcome { session, welcome } => Ok((session, welcome)),
        HandshakePrelude::Content { mut session, offer } => {
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentRequest {
                    full_mod_sha256: offer.full_mod_sha256,
                    resume_offset: offer.encoded_bytes,
                },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "reconnect content request",
            )
            .await?;
            write_frame_with_timeout(
                &mut session.send,
                &NetMsg::ContentReady {
                    full_mod_sha256: offer.full_mod_sha256,
                },
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "reconnect content readiness",
            )
            .await?;
            read_welcome(session).await
        }
    }
}

fn publish_speech_timing_authority(shared: &Mutex<Option<Option<String>>>, locale: Option<String>) {
    *shared.lock() = Some(locale);
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
    transport_key: SecretKey,
    durable_ranked_key: Option<SecretKey>,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_setup_rx: &mut UnboundedReceiver<
        Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    >,
    session_id_shared: Arc<Mutex<Option<MultiplayerSessionId>>>,
    mission_id_shared: Arc<Mutex<Option<String>>>,
    mission_seed_shared: Arc<Mutex<Option<u64>>>,
    mission_config_shared: Arc<Mutex<Option<robin_engine::engine::SimConfig>>>,
    speech_timing_locale_shared: Arc<Mutex<Option<Option<String>>>>,
    content_offer_shared: Arc<Mutex<Option<robin_engine::multiplayer::DistributedModOffer>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<InitialHandshake, String>>,
    cancellation: Arc<AtomicBool>,
) {
    let endpoint = match bind_endpoint(transport_key, GAME_ALPN).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            let _ = initial_handshake_tx.send(Err(e));
            return;
        }
    };

    run_client_io_inner(
        &endpoint,
        durable_ranked_key,
        server_addr,
        nickname,
        incoming_tx,
        outgoing_async_rx,
        assigned,
        ranked_lifecycle,
        ranked_setup_rx,
        session_id_shared,
        mission_id_shared,
        mission_seed_shared,
        mission_config_shared,
        speech_timing_locale_shared,
        content_offer_shared,
        initial_handshake_tx,
        cancellation,
    )
    .await;

    endpoint.close().await;
}

#[allow(clippy::too_many_arguments)]
async fn run_client_io_inner(
    endpoint: &Endpoint,
    durable_ranked_key: Option<SecretKey>,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    assigned: Arc<Mutex<Option<PlayerId>>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_setup_rx: &mut UnboundedReceiver<
        Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    >,
    session_id_shared: Arc<Mutex<Option<MultiplayerSessionId>>>,
    mission_id_shared: Arc<Mutex<Option<String>>>,
    mission_seed_shared: Arc<Mutex<Option<u64>>>,
    mission_config_shared: Arc<Mutex<Option<robin_engine::engine::SimConfig>>>,
    speech_timing_locale_shared: Arc<Mutex<Option<Option<String>>>>,
    content_offer_shared: Arc<Mutex<Option<robin_engine::multiplayer::DistributedModOffer>>>,
    initial_handshake_tx: std::sync::mpsc::SyncSender<Result<InitialHandshake, String>>,
    cancellation: Arc<AtomicBool>,
) {
    let prelude = {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut backoff = std::time::Duration::from_millis(50);
        loop {
            if cancellation.load(Ordering::Acquire) {
                let _ = initial_handshake_tx.send(Err("transport cancelled".into()));
                return;
            }
            let Some(handshake) = handshake_or_cancel(
                endpoint,
                &server_addr,
                &nickname,
                durable_ranked_key
                    .as_ref()
                    .map(|key| *key.public().as_bytes()),
                &cancellation,
            )
            .await
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

    let (mut session, welcome, admitted_offer) = match prelude {
        HandshakePrelude::Welcome { session, welcome } => (session, welcome, None),
        HandshakePrelude::Content { session, offer } => {
            *content_offer_shared.lock() = Some(offer.clone());
            let _ = incoming_tx.send(NetEvent::ContentOffer(offer.clone()));
            let _ = initial_handshake_tx.send(Ok(InitialHandshake::ContentOffered {
                full_mod_sha256: offer.full_mod_sha256,
            }));
            match complete_content_admission(
                session,
                &offer,
                &incoming_tx,
                outgoing_async_rx,
                &cancellation,
            )
            .await
            {
                Ok(ContentAdmissionCompletion::Join(session, welcome)) => {
                    (session, welcome, Some(offer))
                }
                Ok(ContentAdmissionCompletion::Prepared) => {
                    let _ = incoming_tx.send(NetEvent::Note(format!(
                        "verified and cached host content {} without joining a gameplay seat",
                        robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                    )));
                    return;
                }
                Err(error) => {
                    let _ = incoming_tx.send(NetEvent::Fatal(format!(
                        "distributed-mod admission failed: {error}"
                    )));
                    return;
                }
            }
        }
    };
    let WelcomeData {
        seat: your_seat,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        session_id,
    } = welcome;

    *assigned.lock() = Some(your_seat);
    *session_id_shared.lock() = Some(session_id);
    *mission_id_shared.lock() = Some(mission_id.clone());
    *mission_seed_shared.lock() = Some(mission_seed);
    *mission_config_shared.lock() = Some(sim_config);
    // Publish this readiness sentinel last. Observing the outer `Some`
    // therefore means every authoritative Welcome field above is available,
    // including when the host explicitly selected no locale override.
    publish_speech_timing_authority(&speech_timing_locale_shared, speech_timing_locale.clone());
    *content_offer_shared.lock() = admitted_offer.clone();
    if admitted_offer.is_none() {
        let _ = initial_handshake_tx.send(Ok(InitialHandshake::Welcomed {
            seat: your_seat,
            mission_seed,
        }));
    }
    let _ = incoming_tx.send(NetEvent::AssignedLocalSeat(your_seat));
    let _ = incoming_tx.send(NetEvent::MissionConfig {
        mission_id: mission_id.clone(),
        rng_seed: mission_seed,
        sim_config,
        speech_timing_locale: speech_timing_locale.clone(),
    });
    let leaderboard_cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
    let ranked_join_state: SharedClientRankedJoinState = Arc::new(Default::default());
    let ranked_setup_state = Arc::new(AtomicU8::new(RANKED_SETUP_AWAITING));
    let mut backoff = std::time::Duration::from_millis(500);
    loop {
        match run_session_async(
            session,
            your_seat,
            *endpoint.id().as_bytes(),
            *server_addr.id.as_bytes(),
            durable_ranked_key.clone(),
            &ranked_lifecycle,
            &ranked_join_state,
            ranked_setup_rx,
            &ranked_setup_state,
            &incoming_tx,
            outgoing_async_rx,
            &leaderboard_cosign_state,
            &cancellation,
        )
        .await
        {
            SessionEnd::Graceful => break,
            SessionEnd::Drop(reason) => {
                if ranked_lifecycle_lock(&ranked_lifecycle)
                    .browse_only_reason()
                    .is_none()
                    && let Err(error) = ranked_join_state.begin_reconnect()
                {
                    ranked_lifecycle_lock(&ranked_lifecycle).downgrade(format!(
                        "ranked reconnect trust state could not advance: {error}"
                    ));
                }
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
            SessionEnd::Fatal(error) => {
                let _ = incoming_tx.send(NetEvent::Fatal(error));
                return;
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
            let Some(handshake) = handshake_or_cancel(
                endpoint,
                &server_addr,
                &nickname,
                durable_ranked_key
                    .as_ref()
                    .map(|key| *key.public().as_bytes()),
                &cancellation,
            )
            .await
            else {
                return;
            };
            match handshake {
                Ok(prelude) => {
                    let resolved =
                        resolve_reconnect_prelude(prelude, admitted_offer.as_ref()).await;
                    let (new_session, welcome) = match resolved {
                        Ok(result) => result,
                        Err(e) => {
                            tracing::warn!(
                                "reconnect content admission failed: {e}; will retry in {backoff:?}"
                            );
                            if sleep_or_cancel(backoff, &cancellation).await {
                                return;
                            }
                            backoff = (backoff * 2).min(std::time::Duration::from_secs(10));
                            continue;
                        }
                    };
                    let WelcomeData {
                        seat: new_seat,
                        mission_id: new_mission_id,
                        mission_seed: new_seed,
                        sim_config: new_config,
                        speech_timing_locale: new_speech_timing_locale,
                        session_id: new_session_id,
                    } = welcome;
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
    /// A direction, session, request, or signature invariant failed. Retrying
    /// the same authenticated session cannot repair this trust violation.
    Fatal(String),
    /// The game loop dropped the outgoing channel — caller should
    /// stop the I/O thread entirely (no retry).
    OutgoingClosed,
}

#[derive(Clone)]
struct ClientRankedTransportContext {
    local_seat: PlayerId,
    local_transport_endpoint: [u8; 32],
    authenticated_host_endpoint: [u8; 32],
    durable_ranked_key: Option<SecretKey>,
    lifecycle: SharedRankedSessionLifecycle,
    join_state: SharedClientRankedJoinState,
    setup_state: Arc<AtomicU8>,
    response_tx: UnboundedSender<RankedJoinResponse>,
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
    local_seat: PlayerId,
    local_transport_endpoint: [u8; 32],
    authenticated_host_endpoint: [u8; 32],
    durable_ranked_key: Option<SecretKey>,
    ranked_lifecycle: &SharedRankedSessionLifecycle,
    ranked_join_state: &SharedClientRankedJoinState,
    ranked_setup_rx: &mut UnboundedReceiver<
        Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    >,
    ranked_setup_state: &Arc<AtomicU8>,
    incoming_tx: &Sender<NetEvent>,
    outgoing_rx: &mut UnboundedReceiver<NetOutbound>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    cancellation: &AtomicBool,
) -> SessionEnd {
    let ClientSession {
        _conn,
        mut send,
        mut recv,
        protocol: _,
    } = session;
    let (ranked_response_tx, mut ranked_response_rx) = unbounded_channel();
    let ranked_context = ClientRankedTransportContext {
        local_seat,
        local_transport_endpoint,
        authenticated_host_endpoint,
        durable_ranked_key,
        lifecycle: Arc::clone(ranked_lifecycle),
        join_state: Arc::clone(ranked_join_state),
        setup_state: Arc::clone(ranked_setup_state),
        response_tx: ranked_response_tx,
    };
    let reader_ranked_context = ranked_context.clone();
    let reader = async move {
        loop {
            match read_frame(&mut recv, InboundFramePolicy::ServerToClient).await {
                Ok(Some(msg)) => {
                    if let Err(error) = handle_client_wire_msg(
                        incoming_tx,
                        leaderboard_cosign_state,
                        Some(&reader_ranked_context),
                        msg,
                    ) {
                        if error.starts_with("host requires a full-snapshot reconnect:") {
                            return SessionEnd::Drop(error);
                        }
                        return SessionEnd::Fatal(error);
                    }
                }
                Ok(None) => return SessionEnd::Graceful,
                Err(e) => return SessionEnd::Drop(e),
            }
        }
    };
    let writer_ranked_context = ranked_context;
    let writer = async {
        enum WriterCommand {
            Outbound(NetOutbound),
            RankedResponse(RankedJoinResponse),
            RankedSetup(Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>),
        }
        loop {
            let command = tokio::select! {
                outgoing = outgoing_rx.recv() => {
                    let Some(outgoing) = outgoing else {
                        return SessionEnd::OutgoingClosed;
                    };
                    WriterCommand::Outbound(outgoing)
                }
                response = ranked_response_rx.recv() => {
                    let Some(response) = response else {
                        return SessionEnd::Fatal(
                            "ranked response queue closed while the client session is live".to_string()
                        );
                    };
                    WriterCommand::RankedResponse(response)
                }
                setup = ranked_setup_rx.recv() => {
                    let Some(setup) = setup else {
                        return SessionEnd::Fatal(
                            "ranked setup channel closed while the client session is live".to_string()
                        );
                    };
                    WriterCommand::RankedSetup(setup)
                }
            };
            let outgoing = match command {
                WriterCommand::RankedResponse(response) => {
                    if let Err(error) =
                        write_frame(&mut send, &NetMsg::RankedJoinResponse(response)).await
                    {
                        return SessionEnd::Drop(error);
                    }
                    continue;
                }
                WriterCommand::RankedSetup(setup) => {
                    handle_client_ranked_setup(&writer_ranked_context, setup);
                    continue;
                }
                WriterCommand::Outbound(outgoing) => outgoing,
            };
            let leaderboard_control = matches!(
                &outgoing,
                NetOutbound::LeaderboardCoSignRequest { .. }
                    | NetOutbound::ArmLeaderboardCoSignRequest { .. }
                    | NetOutbound::LeaderboardCoSignResponse(_)
            );
            if let Err(error) = send_client_outgoing(
                &mut send,
                outgoing,
                incoming_tx,
                leaderboard_cosign_state,
                &writer_ranked_context,
            )
            .await
            {
                return if leaderboard_control {
                    SessionEnd::Fatal(error)
                } else {
                    SessionEnd::Drop(error)
                };
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

fn downgrade_client_ranked(
    context: &ClientRankedTransportContext,
    reason: RankedBrowseOnlyReason,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    ranked_lifecycle_lock(&context.lifecycle).downgrade(detail.clone());
    tracing::warn!(?reason, %detail, "client ranked admission downgraded; gameplay remains available");
}

fn handle_client_ranked_setup(
    context: &ClientRankedTransportContext,
    setup: Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
) {
    let Some(setup) = setup else {
        context
            .setup_state
            .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
        let _ = queue_client_ranked_response(
            context,
            RankedJoinResponse::Unavailable(
                super::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
            ),
        );
        downgrade_client_ranked(
            context,
            RankedBrowseOnlyReason::PeerRankedSessionMismatch,
            "local prepared inputs explicitly selected browse-only multiplayer",
        );
        return;
    };
    let expected = setup.ranked_session.clone();
    let official_subject =
        robin_run_protocol::official_content_subjects_v1(expected.content_edition)
            .contains(&expected.content_subject);
    let document = encode_ranked_wire_document(&expected)
        .map_err(|error| error.to_string())
        .and_then(|bytes| super::RankedSessionConfigDocument::new(bytes).map_err(str::to_string));
    let admission = context.durable_ranked_key.as_ref().map(|key| {
        RankedSessionClientAdmissionV1::new_official(
            setup,
            PublicKey32::from_bytes(context.authenticated_host_endpoint),
            PublicKey32::from_bytes(*key.public().as_bytes()),
            PublicKey32::from_bytes(context.local_transport_endpoint),
        )
    });
    let (document, admission) = match (official_subject, document, admission) {
        (true, Ok(document), Some(Ok(admission))) => (document, admission),
        (true, Ok(document), None) => {
            context
                .setup_state
                .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
            if let Ok(Some(challenge)) = context.join_state.arm_expected_session(document) {
                handle_delivered_ranked_challenge(context, challenge);
            }
            downgrade_client_ranked(
                context,
                RankedBrowseOnlyReason::PeerIdentityUnavailable,
                "durable ranked identity is unavailable",
            );
            return;
        }
        (_, document, admission) => {
            context
                .setup_state
                .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
            let detail = format!(
                "official ranked client setup failed: subject_official={official_subject}, document={:?}, admission={:?}",
                document.err(),
                admission.and_then(Result::err)
            );
            let _ = queue_client_ranked_response(
                context,
                RankedJoinResponse::Unavailable(
                    super::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                ),
            );
            downgrade_client_ranked(
                context,
                RankedBrowseOnlyReason::PeerRankedSessionMismatch,
                detail,
            );
            return;
        }
    };
    if let Err(error) =
        ranked_lifecycle_lock(&context.lifecycle).install_client_admission(admission)
    {
        context
            .setup_state
            .store(RANKED_SETUP_UNAVAILABLE, Ordering::Release);
        downgrade_client_ranked(
            context,
            RankedBrowseOnlyReason::RankedProtocolViolation,
            format!("could not install ranked client admission: {error}"),
        );
        return;
    }
    context
        .setup_state
        .store(RANKED_SETUP_AVAILABLE, Ordering::Release);
    match context.join_state.arm_expected_session(document) {
        Ok(Some(challenge)) => handle_delivered_ranked_challenge(context, challenge),
        Ok(None) => {}
        Err(error) => downgrade_client_ranked(
            context,
            RankedBrowseOnlyReason::RankedProtocolViolation,
            error,
        ),
    }
}

fn queue_client_ranked_response(
    context: &ClientRankedTransportContext,
    response: RankedJoinResponse,
) -> Result<(), String> {
    context.join_state.authorize_response(&response)?;
    context
        .response_tx
        .send(response)
        .map_err(|_| "ranked response writer queue is closed".to_string())
}

fn respond_to_ranked_challenge(
    context: &ClientRankedTransportContext,
    challenge: RankedJoinChallenge,
) -> Result<(), String> {
    let Some(durable_key) = context.durable_ranked_key.as_ref() else {
        return queue_client_ranked_response(
            context,
            RankedJoinResponse::Unavailable(
                super::RankedJoinUnavailableReason::DurableIdentityUnavailable,
            ),
        );
    };
    let claim: robin_run_protocol::NamedSeatJoinClaimV1 =
        decode_ranked_wire_document(challenge.join_claim.as_bytes())
            .map_err(|error| format!("invalid ranked join claim: {error}"))?;
    let genesis: robin_run_protocol::ReplaySessionGenesisV1 =
        decode_ranked_wire_document(challenge.session_genesis.as_bytes())
            .map_err(|error| format!("invalid ranked session genesis: {error}"))?;
    let expected_setup = {
        let lifecycle = ranked_lifecycle_lock(&context.lifecycle);
        lifecycle
            .client_admission()
            .map(|admission| admission.expected_setup.clone())
            .or_else(|| {
                lifecycle
                    .ranked_client()
                    .map(|client| client.admission.expected_setup.clone())
            })
    }
    .ok_or_else(|| {
        "ranked challenge arrived without locally installed client inputs".to_string()
    })?;
    crate::leaderboard_ranked_session::validate_official_session_genesis(
        &genesis,
        context.authenticated_host_endpoint,
        &expected_setup,
    )
    .map_err(|error| format!("ranked challenge host/session mismatch: {error}"))?;
    if claim.public_key != PublicKey32::from_bytes(*durable_key.public().as_bytes())
        || claim.transport_endpoint_id != PublicKey32::from_bytes(context.local_transport_endpoint)
        || claim.host_endpoint_id != PublicKey32::from_bytes(context.authenticated_host_endpoint)
        || claim.seat != u16::from(context.local_seat.0)
    {
        return Err(
            "ranked join claim does not bind this durable client transport/seat".to_string(),
        );
    }
    let durable_signing_key = ed25519_dalek::SigningKey::from_bytes(&durable_key.to_bytes());
    let attestation = sign_named_seat_join(&durable_signing_key, claim)
        .map_err(|error| format!("sign ranked named-seat claim: {error}"))?;
    let bytes = encode_ranked_wire_document(&attestation)
        .map_err(|error| format!("encode ranked named-seat attestation: {error}"))?;
    let document = RankedJoinAttestationDocument::new(bytes)
        .map_err(|error| format!("encode ranked named-seat attestation: {error}"))?;
    queue_client_ranked_response(context, RankedJoinResponse::Attestation(document))
}

fn handle_delivered_ranked_challenge(
    context: &ClientRankedTransportContext,
    challenge: RankedJoinChallenge,
) {
    if let Err(error) = respond_to_ranked_challenge(context, challenge) {
        let unavailable = RankedJoinResponse::Unavailable(
            super::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
        );
        if let Err(queue_error) = queue_client_ranked_response(context, unavailable) {
            tracing::warn!(%queue_error, "could not send typed ranked admission unavailability");
        }
        downgrade_client_ranked(
            context,
            RankedBrowseOnlyReason::PeerRankedSessionMismatch,
            error,
        );
    }
}

fn accept_client_ranked_join(
    context: &ClientRankedTransportContext,
    accepted: RankedJoinAccepted,
) -> Result<RankedJoinAccepted, String> {
    let accepted = context.join_state.receive_wire_acceptance(accepted)?;
    let genesis: robin_run_protocol::ReplaySessionGenesisV1 =
        decode_ranked_wire_document(accepted.session_genesis.as_bytes())
            .map_err(|error| format!("invalid accepted ranked genesis: {error}"))?;
    let attestation: robin_run_protocol::NamedSeatJoinAttestationV1 =
        decode_ranked_wire_document(accepted.join_attestation.as_bytes())
            .map_err(|error| format!("invalid accepted ranked attestation: {error}"))?;
    let participant_claims =
        super::decode_ranked_participant_roster(&accepted.participant_roster, &genesis)?;
    let mut lifecycle = ranked_lifecycle_lock(&context.lifecycle);
    if let Some(client) = lifecycle.ranked_client() {
        if client.session_genesis != genesis
            || client.local_seat != u16::from(context.local_seat.0)
            || attestation.claim.public_key != client.admission.local_public_key
            || attestation.claim.participant_instance_id
                != client
                    .participant_claims
                    .iter()
                    .find(|participant| participant.seat == client.local_seat)
                    .ok_or_else(|| "ranked client roster lost its local participant".to_string())?
                    .participant_instance_id
        {
            return Err("ranked reconnect acknowledgement changed admitted client identity".into());
        }
        lifecycle
            .update_ranked_client_roster(&genesis, participant_claims)
            .map_err(|error| format!("update ranked client roster after reconnect: {error}"))?;
        return Ok(accepted);
    }
    lifecycle
        .accept_ranked_client(u16::from(context.local_seat.0), genesis, participant_claims)
        .map_err(|error| format!("accept ranked client lifecycle: {error}"))?;
    Ok(accepted)
}

fn handle_client_wire_msg(
    incoming_tx: &Sender<NetEvent>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    ranked_context: Option<&ClientRankedTransportContext>,
    msg: NetMsg,
) -> Result<(), String> {
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
            if let Some(context) = ranked_context {
                let unresolved = !context.join_state.is_accepted()?
                    && ranked_lifecycle_lock(&context.lifecycle)
                        .browse_only_reason()
                        .is_none();
                if unresolved {
                    let _ = queue_client_ranked_response(
                        context,
                        RankedJoinResponse::Unavailable(
                            super::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                        ),
                    );
                    let _ = context
                        .join_state
                        .mark_browse_only(RankedBrowseOnlyReason::RankedProtocolViolation);
                    downgrade_client_ranked(
                        context,
                        RankedBrowseOnlyReason::RankedProtocolViolation,
                        "host released simulation before ranked admission or browse-only resolution",
                    );
                    let _ = incoming_tx.send(NetEvent::RankedBrowseOnly {
                        reason: RankedBrowseOnlyReason::RankedProtocolViolation,
                    });
                }
            }
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
        NetMsg::LeaderboardCoSignRequest(request) => {
            if let Some(ranked_context) = ranked_context
                && ranked_lifecycle_lock(&ranked_context.lifecycle)
                    .ranked_client()
                    .is_none()
            {
                downgrade_client_ranked(
                    ranked_context,
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    "host requested leaderboard co-signing before ranked client admission",
                );
                return Ok(());
            }
            if let Some(request) = leaderboard_cosign_state.receive_wire_request(request)? {
                incoming_tx
                    .send(NetEvent::LeaderboardCoSignRequest(request))
                    .map_err(|_| {
                        "client leaderboard co-sign request channel is closed".to_string()
                    })?;
            }
        }
        NetMsg::LeaderboardCoSignResponse(_) => {
            return Err("server sent a client-only leaderboard co-sign response".into());
        }
        NetMsg::RankedJoinChallenge(challenge) => {
            let context = ranked_context
                .ok_or_else(|| "ranked challenge has no native client trust context".to_string())?;
            match context.join_state.receive_wire_challenge(challenge) {
                Ok(Some(challenge))
                    if context.setup_state.load(Ordering::Acquire) == RANKED_SETUP_AVAILABLE =>
                {
                    handle_delivered_ranked_challenge(context, challenge);
                }
                Ok(_)
                    if context.setup_state.load(Ordering::Acquire) == RANKED_SETUP_UNAVAILABLE =>
                {
                    if let Err(error) = queue_client_ranked_response(
                        context,
                        RankedJoinResponse::Unavailable(
                            super::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
                        ),
                    ) {
                        tracing::warn!(%error, "could not answer ranked challenge with typed unavailability");
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = queue_client_ranked_response(
                        context,
                        RankedJoinResponse::Unavailable(
                            super::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                        ),
                    );
                    downgrade_client_ranked(
                        context,
                        RankedBrowseOnlyReason::PeerRankedSessionMismatch,
                        error,
                    );
                }
            }
        }
        NetMsg::RankedJoinAccepted(accepted) => {
            let context = ranked_context.ok_or_else(|| {
                "ranked acknowledgement has no native client trust context".to_string()
            })?;
            match accept_client_ranked_join(context, accepted) {
                Ok(accepted) => {
                    incoming_tx
                        .send(NetEvent::RankedJoinAccepted(accepted))
                        .map_err(|_| {
                            "client ranked acknowledgement channel is closed".to_string()
                        })?;
                }
                Err(error) => downgrade_client_ranked(
                    context,
                    RankedBrowseOnlyReason::PeerAttestationRejected,
                    error,
                ),
            }
        }
        NetMsg::RankedParticipantRoster(document) => {
            let context = ranked_context.ok_or_else(|| {
                "ranked roster update has no native client trust context".to_string()
            })?;
            let update = (|| {
                let document = context.join_state.receive_wire_roster_update(document)?;
                let genesis = ranked_lifecycle_lock(&context.lifecycle)
                    .ranked_client()
                    .map(|client| client.session_genesis.clone())
                    .ok_or_else(|| {
                        "ranked roster update arrived before client lifecycle acceptance"
                            .to_string()
                    })?;
                let participant_claims =
                    super::decode_ranked_participant_roster(&document, &genesis)?;
                ranked_lifecycle_lock(&context.lifecycle)
                    .update_ranked_client_roster(&genesis, participant_claims)
                    .map_err(|error| format!("update ranked client roster: {error}"))?;
                Ok::<_, String>(document)
            })();
            match update {
                Ok(document) => incoming_tx
                    .send(NetEvent::RankedParticipantRoster(document))
                    .map_err(|_| "client ranked roster channel is closed".to_string())?,
                Err(error) => downgrade_client_ranked(
                    context,
                    RankedBrowseOnlyReason::PeerAttestationRejected,
                    error,
                ),
            }
        }
        NetMsg::RankedBrowseOnly { reason } => {
            let context = ranked_context.ok_or_else(|| {
                "ranked browse-only event has no native client trust context".to_string()
            })?;
            let _ = context.join_state.mark_browse_only(reason);
            ranked_lifecycle_lock(&context.lifecycle)
                .downgrade(format!("host downgraded ranked multiplayer: {reason:?}"));
            incoming_tx
                .send(NetEvent::RankedBrowseOnly { reason })
                .map_err(|_| "client ranked browse-only channel is closed".to_string())?;
        }
        NetMsg::RankedOfficialSessionSetup(document) => {
            crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                OfficialRankedSessionWireSetupV1,
            >(document.as_bytes())
            .map_err(|error| format!("invalid official ranked wire setup: {error}"))?;
            incoming_tx
                .send(NetEvent::RankedOfficialSessionSetup(document))
                .map_err(|_| "client official ranked setup channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationReceiptSelectionRequest(document) => {
            let context = ranked_context.ok_or_else(|| {
                "continuation receipt selection request has no native client trust context"
                    .to_string()
            })?;
            let request = decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionRequestV1,
            >(document.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection request: {error}"))?;
            let local_key = context.durable_ranked_key.as_ref().ok_or_else(|| {
                "continuation receipt selection has no durable ranked identity".to_string()
            })?;
            let local_public_key = PublicKey32::from_bytes(*local_key.public().as_bytes());
            if request.lobby.host_public_key
                != PublicKey32::from_bytes(context.authenticated_host_endpoint)
                || request
                    .lobby
                    .participant_public_keys
                    .binary_search(&local_public_key)
                    .is_err()
            {
                return Err(
                    "continuation receipt selection request does not bind the authenticated host and local peer"
                        .to_string(),
                );
            }
            incoming_tx
                .send(NetEvent::RankedContinuationReceiptSelectionRequest(
                    document,
                ))
                .map_err(|_| {
                    "client continuation receipt selection request channel is closed".to_string()
                })?;
        }
        NetMsg::RankedContinuationReceiptSelection(_) => {
            return Err("server sent a client-only continuation receipt selection".into());
        }
        NetMsg::RankedContinuationPreflightClaim(document) => {
            let context = ranked_context.ok_or_else(|| {
                "continuation preflight claim has no native client trust context".to_string()
            })?;
            let claim = decode_ranked_wire_document::<CampaignContinuationPreflightRequestClaimV1>(
                document.as_bytes(),
            )
            .map_err(|error| format!("invalid continuation preflight claim: {error}"))?;
            let local_key = context.durable_ranked_key.as_ref().ok_or_else(|| {
                "continuation preflight controller has no durable ranked identity".to_string()
            })?;
            if claim.host_public_key != PublicKey32::from_bytes(context.authenticated_host_endpoint)
                || claim.campaign_controller_public_key
                    != PublicKey32::from_bytes(*local_key.public().as_bytes())
            {
                return Err(
                    "continuation preflight claim does not bind the authenticated host and local controller"
                        .to_string(),
                );
            }
            incoming_tx
                .send(NetEvent::RankedContinuationPreflightClaim(document))
                .map_err(|_| "client continuation preflight claim channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationPreflightSignature(_) => {
            return Err("server sent a client-only continuation preflight signature".into());
        }
        NetMsg::RankedCoSignContext(context) => {
            let ranked_context = ranked_context.ok_or_else(|| {
                "ranked co-sign context has no native client trust context".to_string()
            })?;
            let decoded = decode_ranked_wire_document::<
                crate::leaderboard_ranked_session::RankedCoSignContextV1,
            >(context.as_bytes());
            if let Err(error) = decoded {
                downgrade_client_ranked(
                    ranked_context,
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    format!("host published invalid ranked co-sign context: {error}"),
                );
            } else if ranked_lifecycle_lock(&ranked_context.lifecycle)
                .ranked_client()
                .is_some()
            {
                incoming_tx
                    .send(NetEvent::RankedCoSignContext(context))
                    .map_err(|_| "client ranked context channel is closed".to_string())?;
            } else {
                downgrade_client_ranked(
                    ranked_context,
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    "host published ranked co-sign context before client admission",
                );
            }
        }
        NetMsg::RankedSubmissionAccepted(accepted) => {
            let ranked_context = ranked_context.ok_or_else(|| {
                "ranked submission acknowledgement has no native client trust context".to_string()
            })?;
            let decoded = decode_ranked_wire_document::<robin_run_protocol::SubmissionAcceptedV1>(
                accepted.as_bytes(),
            );
            if let Err(error) = decoded {
                downgrade_client_ranked(
                    ranked_context,
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    format!("host published invalid ranked submission acknowledgement: {error}"),
                );
            } else if ranked_lifecycle_lock(&ranked_context.lifecycle)
                .ranked_client()
                .is_some()
            {
                incoming_tx
                    .send(NetEvent::RankedSubmissionAccepted(accepted))
                    .map_err(|_| {
                        "client ranked submission acknowledgement channel is closed".to_string()
                    })?;
            } else {
                downgrade_client_ranked(
                    ranked_context,
                    RankedBrowseOnlyReason::RankedProtocolViolation,
                    "host published a submission acknowledgement before ranked client admission",
                );
            }
        }
        NetMsg::RankedJoinResponse(_) => {
            return Err("server sent a client-only ranked join response".into());
        }
        NetMsg::Reject { reason } => return Err(format!("host rejected session: {reason}")),
        other => {
            return Err(format!(
                "host sent invalid native session message {other:?}"
            ));
        }
    }
    Ok(())
}

async fn send_client_outgoing(
    send: &mut SendStream,
    outgoing: NetOutbound,
    incoming_tx: &Sender<NetEvent>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    ranked_context: &ClientRankedTransportContext,
) -> Result<(), String> {
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
            return Err("native client attempted a host-only state hash publication".to_owned());
        }
        NetOutbound::InitialSnapshot { .. } => {
            return Err(
                "native client attempted a host-only initial snapshot publication".to_owned(),
            );
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
            return Err("native client attempted an authoritative modal decision".to_owned());
        }
        NetOutbound::ReconnectForSnapshot { reason, .. }
        | NetOutbound::ReconnectAllForSnapshot { reason } => {
            return Err(format!(
                "full-snapshot resynchronization requested: {reason}"
            ));
        }
        NetOutbound::BeginSnapshotTransition { .. } => {
            return Err("native client attempted an authoritative snapshot transition".to_owned());
        }
        NetOutbound::SnapshotTransitionReady { id } => {
            write_frame(send, &NetMsg::SnapshotTransitionReady { id }).await?;
        }
        NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. } => {
            return Err(
                "native client queued a content-admission message after gameplay began".to_owned(),
            );
        }
        NetOutbound::RankedJoinResponse(_)
        | NetOutbound::ArmRankedJoin { .. }
        | NetOutbound::RankedBrowseOnly { .. } => {
            tracing::error!(
                "native client ignored ranked admission control outside its typed setup/signer seam"
            );
        }
        NetOutbound::RankedJoinChallenge { .. }
        | NetOutbound::RankedJoinAccepted { .. }
        | NetOutbound::RankedParticipantRoster { .. }
        | NetOutbound::RankedOfficialSessionSetup(_)
        | NetOutbound::RankedContinuationReceiptSelectionRequest(_)
        | NetOutbound::RankedContinuationPreflightClaim { .. }
        | NetOutbound::RankedCoSignContext { .. }
        | NetOutbound::RankedSubmissionAccepted { .. } => {
            return Err("native client attempted a server-only ranked control message".to_owned());
        }
        NetOutbound::LeaderboardCoSignRequest { .. } => {
            return Err("client attempted a server-only leaderboard co-sign request".to_string());
        }
        NetOutbound::ArmLeaderboardCoSignRequest { request } => {
            if ranked_lifecycle_lock(&ranked_context.lifecycle)
                .ranked_client()
                .is_none()
            {
                tracing::warn!("ignored leaderboard co-sign arm outside ranked client session");
                return Ok(());
            }
            if let Some(request) = leaderboard_cosign_state.arm_request(request)? {
                incoming_tx
                    .send(NetEvent::LeaderboardCoSignRequest(request))
                    .map_err(|_| {
                        "client leaderboard co-sign request channel is closed".to_string()
                    })?;
            }
        }
        NetOutbound::LeaderboardCoSignResponse(response) => {
            if ranked_lifecycle_lock(&ranked_context.lifecycle)
                .ranked_client()
                .is_none()
            {
                tracing::warn!(
                    "ignored leaderboard co-sign response outside ranked client session"
                );
                return Ok(());
            }
            leaderboard_cosign_state.authorize_response(&response)?;
            write_frame(send, &NetMsg::LeaderboardCoSignResponse(response)).await?;
        }
        NetOutbound::RankedContinuationReceiptSelection(selection) => {
            let decoded = decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionResponseV1,
            >(selection.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
            let durable_key = ranked_context.durable_ranked_key.as_ref().ok_or_else(|| {
                "continuation receipt selection has no durable ranked identity".to_string()
            })?;
            if decoded.responder_public_key()
                != PublicKey32::from_bytes(*durable_key.public().as_bytes())
            {
                return Err(
                    "continuation receipt selection is controlled by another durable identity"
                        .to_string(),
                );
            }
            write_frame(send, &NetMsg::RankedContinuationReceiptSelection(selection)).await?;
        }
        NetOutbound::RankedContinuationPreflightSignature(signature) => {
            let signed =
                crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                    ParticipantSignatureV1,
                >(signature.as_bytes())
                .map_err(|error| format!("invalid continuation preflight signature: {error}"))?;
            if signed.public_key.is_zero() || signed.signature.is_zero() {
                return Err(
                    "continuation preflight signature contains zero key material".to_string(),
                );
            }
            let durable_key = ranked_context.durable_ranked_key.as_ref().ok_or_else(|| {
                "continuation preflight response has no durable ranked identity".to_string()
            })?;
            if signed.public_key != PublicKey32::from_bytes(*durable_key.public().as_bytes()) {
                return Err(
                    "continuation preflight response uses a key other than the authenticated client identity"
                        .to_string(),
                );
            }
            write_frame(
                send,
                &NetMsg::RankedContinuationPreflightSignature(signature),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
fn client_gameplay_wire_msg(outgoing: NetOutbound) -> Result<NetMsg, String> {
    match outgoing {
        NetOutbound::Input {
            origin_frame,
            command,
        } => Ok(NetMsg::Input {
            origin_frame,
            command,
        }),
        NetOutbound::ReadyToSim { frame } => Ok(NetMsg::ReadyToSim { frame }),
        NetOutbound::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        } => Ok(NetMsg::ModalProposal {
            instance,
            kind,
            result,
            requested_frame,
        }),
        NetOutbound::SnapshotTransitionReady { id } => Ok(NetMsg::SnapshotTransitionReady { id }),
        NetOutbound::StateHash { .. } | NetOutbound::InitialSnapshot { .. } => {
            Err("native client attempted a host-only multiplayer publication".to_owned())
        }
        NetOutbound::ContentRequest { .. }
        | NetOutbound::ContentReject { .. }
        | NetOutbound::ContentReady { .. }
        | NetOutbound::ContentPrepared { .. } => {
            Err("native client queued a content-admission message after gameplay began".to_owned())
        }
        _ => Err("outbound message is not a direct native gameplay frame".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostSessionContinuation, PeerOwner, PendingSnapshotTransition, SeatClaimKind, ServerPeers,
        SharedClientLeaderboardCoSignState, checked_epoch_ms, client_gameplay_wire_msg,
        connect_client_with_keys, discard_session_outbound, handle_client_wire_msg,
        publish_speech_timing_authority, retain_transition_peer_for_reconnect,
        start_server_with_key, take_committed_snapshot_transition, validate_peer_command_authority,
        validate_reconnect_session_id, validate_reconnect_state, validate_server_gameplay_outbound,
        validate_server_gameplay_wire_msg,
    };
    use crate::leaderboard_ranked_session::{
        OfficialRankedSessionSetupV1, RankedRunPreflightAdmissionV1,
    };
    use crate::multiplayer::{MAX_CONTENT_FRAME_BYTES, MAX_SERVER_CONTROL_FRAME_BYTES};
    use ed25519_dalek::{Signer, SigningKey};
    use robin_engine::multiplayer::LeaderboardCoSignResponse;
    use robin_engine::multiplayer::{NetEvent, NetOutbound};
    use robin_engine::player_command::PlayerId;
    use robin_run_protocol::{
        ArtifactRefV1, CanonicalDocument as _, ChallengeNonce32, Digest32,
        FreshRunPreflightGrantClaimV1, FreshRunPreflightGrantV1, FreshRunPreflightRequestClaimV1,
        FreshRunPreflightRequestV1, FreshRunScopeV1, LeaderboardCoSignInstanceV1,
        LeaderboardCoSignPurposeV1, LeaderboardCoSignRequestV1, OfficialContentEditionV1,
        OfficialContentSubjectV1, OpaqueId, PublicKey32, RANKED_CAMPAIGN_MEDIA_TYPE_V1,
        RankedSessionConfigV1, ResourceLocaleRootV1, SCHEMA_VERSION_V1, Signature64,
        SignatureAlgorithmV1, SimulationSeed64, SpeechTimingAuthorityV1, SubmissionAcceptedV1,
        SubmissionLifecycleV1,
    };
    use std::collections::HashSet;
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::unbounded_channel;

    fn ranked_identity(byte: u8) -> super::RankedPeerIdentity {
        super::RankedPeerIdentity {
            durable_public_key: Some([byte; 32]),
            transport_endpoint_id: [byte.wrapping_add(1); 32],
            public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
        }
    }

    fn leaderboard_request(
        purpose: LeaderboardCoSignPurposeV1,
        byte: u8,
    ) -> LeaderboardCoSignRequestV1 {
        LeaderboardCoSignRequestV1 {
            instance: LeaderboardCoSignInstanceV1 {
                purpose,
                replay_session_id: Digest32::from_bytes([byte; 32]),
                submission_offer_sha256: Digest32::from_bytes([byte.wrapping_add(1); 32]),
            },
            run_digest: Digest32::from_bytes([byte.wrapping_add(2); 32]),
        }
    }

    fn official_ranked_setup(host_key: &iroh::SecretKey) -> OfficialRankedSessionSetupV1 {
        let ranked_session = RankedSessionConfigV1 {
            schema_version: SCHEMA_VERSION_V1,
            mission_id: "Dem_Lei_MP".to_string(),
            content_edition: OfficialContentEditionV1::Demo,
            content_subject: OfficialContentSubjectV1::FieldMission {
                mission_id: "Dem_Lei_MP".to_string(),
            },
            simulation_seed: SimulationSeed64::new(7),
            starting_campaign_sha256: Digest32::from_bytes([1; 32]),
            starting_campaign_byte_length: 1,
            prepared_inputs_projection_sha256: Digest32::from_bytes([2; 32]),
            prepared_mission_inputs_seal_sha256: Digest32::from_bytes([3; 32]),
            build_manifest_sha256: Digest32::from_bytes([4; 32]),
            content_manifest_sha256: Digest32::from_bytes([5; 32]),
            campaign_content_manifest_sha256: None,
            rules_config_sha256: Digest32::from_bytes([6; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([7; 32]),
            competition_manifest_sha256: None,
            spellforge_content_sha256: None,
            resource_locale_root: ResourceLocaleRootV1::new("1033").unwrap(),
            speech_timing: SpeechTimingAuthorityV1::BaseInstallation,
        };
        let host_signing_key = SigningKey::from_bytes(&host_key.to_bytes());
        let authority_key = SigningKey::from_bytes(&[0x71; 32]);
        let request_claim = FreshRunPreflightRequestClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            request_nonce: ChallengeNonce32::from_bytes([0x72; 32]),
            host_public_key: PublicKey32::from_bytes(host_signing_key.verifying_key().to_bytes()),
            replay_session_id: Digest32::from_bytes([0x73; 32]),
            host_participant_instance_id: Digest32::from_bytes([0x74; 32]),
            host_nonce: ChallengeNonce32::from_bytes([0x75; 32]),
            scope: FreshRunScopeV1::IndividualLevel,
            starting_campaign: ArtifactRefV1 {
                sha256: ranked_session.starting_campaign_sha256,
                byte_length: ranked_session.starting_campaign_byte_length,
                media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_string(),
            },
            ranked_session: ranked_session.clone(),
        };
        let request = FreshRunPreflightRequestV1 {
            host_signature: Signature64::from_bytes(
                host_signing_key
                    .sign(&request_claim.signing_bytes().unwrap())
                    .to_bytes(),
            ),
            claim: request_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        let grant_claim = FreshRunPreflightGrantClaimV1 {
            schema_version: SCHEMA_VERSION_V1,
            grant_id: OpaqueId::new("native-ranked-test-grant").unwrap(),
            grant_nonce: ChallengeNonce32::from_bytes([0x76; 32]),
            grant_authority_public_key: PublicKey32::from_bytes(
                authority_key.verifying_key().to_bytes(),
            ),
            host_public_key: request.claim.host_public_key,
            grant_request_sha256: request.canonical_digest().unwrap(),
            ranked_session_sha256: ranked_session.canonical_digest().unwrap(),
            replay_session_id: request.claim.replay_session_id,
            host_participant_instance_id: request.claim.host_participant_instance_id,
            host_nonce: request.claim.host_nonce,
            scope: request.claim.scope,
            starting_campaign: request.claim.starting_campaign.clone(),
            admitted_at_unix_ms: 1_000,
            expires_at_unix_ms: 2_000,
        };
        let grant = FreshRunPreflightGrantV1 {
            authority_signature: Signature64::from_bytes(
                authority_key
                    .sign(&grant_claim.signing_bytes().unwrap())
                    .to_bytes(),
            ),
            claim: grant_claim,
            algorithm: SignatureAlgorithmV1::Ed25519,
        };
        OfficialRankedSessionSetupV1 {
            ranked_session,
            custom_package_present: false,
            run_preflight: RankedRunPreflightAdmissionV1::Fresh { request, grant },
            run_preflight_grant_public_key: PublicKey32::from_bytes(
                authority_key.verifying_key().to_bytes(),
            ),
            trusted_now_unix_ms: 1_500,
        }
    }

    #[track_caller]
    fn recv_matching(
        receiver: &Receiver<NetEvent>,
        timeout: Duration,
        mut predicate: impl FnMut(&NetEvent) -> bool,
    ) -> NetEvent {
        let deadline = Instant::now() + timeout;
        let mut skipped = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = match receiver.recv_timeout(remaining) {
                Ok(event) => event,
                Err(error) => {
                    panic!("timed out waiting for multiplayer event: {error}; observed {skipped:?}")
                }
            };
            if predicate(&event) {
                return event;
            }
            skipped.push(event);
        }
    }

    fn signed_response(
        request: &LeaderboardCoSignRequestV1,
        key: &iroh::SecretKey,
    ) -> LeaderboardCoSignResponse {
        LeaderboardCoSignResponse {
            instance: request.instance,
            signer_public_key: *key.public().as_bytes(),
            signature: key.sign(&request.signing_bytes().unwrap()).to_bytes(),
        }
    }

    fn admit_ranked_test_identity(peers: &mut ServerPeers, seat: u8, key: &iroh::SecretKey) {
        peers.ranked_identities.insert(
            seat,
            super::RankedPeerIdentity {
                durable_public_key: Some(*key.public().as_bytes()),
                transport_endpoint_id: *key.public().as_bytes(),
                public_disclosure: robin_run_protocol::ParticipantPublicDisclosureV1::NamedProfile,
            },
        );
    }

    fn offer() -> robin_engine::multiplayer::DistributedModOffer {
        robin_engine::multiplayer::DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 1,
            mission_basename: "Mission".into(),
            mission_rhm_entry: "Data/Levels/Mission.rhm".into(),
            map_filename: "Mission".into(),
            title: "Mission".into(),
            claimed_author: "Author".into(),
            version: "1".into(),
            source_url: "https://example.invalid".into(),
            license: "CC0-1.0".into(),
            host_endpoint_id: "endpoint-key".into(),
        }
    }

    #[test]
    fn admission_phase_caps_fit_max_valid_offer_and_chunk() {
        let text = "x".repeat(robin_engine::multiplayer::DistributedModOffer::TEXT_BYTE_LIMIT);
        let host_id = "x".repeat(
            robin_engine::multiplayer::DistributedModOffer::AUTHENTICATED_HOST_ID_BYTE_LIMIT,
        );
        let offer = robin_engine::multiplayer::DistributedModOffer {
            schema_version: 1,
            full_mod_sha256: [1; 32],
            spellforge_package_sha256: None,
            spellforge_vm_abi: None,
            encoded_bytes: 1,
            mission_basename: text.clone(),
            mission_rhm_entry: text.clone(),
            map_filename: text.clone(),
            title: text.clone(),
            claimed_author: text.clone(),
            version: text.clone(),
            source_url: text.clone(),
            license: text.clone(),
            host_endpoint_id: host_id,
        };
        offer.validate().unwrap();
        let offer_bytes = super::encode_msg(&super::NetMsg::ContentOffer { offer });
        assert!(offer_bytes.len() <= MAX_SERVER_CONTROL_FRAME_BYTES);

        let chunk_bytes = super::encode_msg(&super::NetMsg::ContentChunk {
            full_mod_sha256: [1; 32],
            offset: 0,
            total_bytes: robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT as u64,
            bytes: vec![0; robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT],
        });
        assert!(chunk_bytes.len() <= MAX_CONTENT_FRAME_BYTES);
        assert!(chunk_bytes.len() > MAX_SERVER_CONTROL_FRAME_BYTES);
    }

    #[test]
    fn speech_timing_readiness_distinguishes_pending_some_and_none() {
        let with_locale = parking_lot::Mutex::new(None);
        assert_eq!(*with_locale.lock(), None);
        publish_speech_timing_authority(&with_locale, Some("en-US".into()));
        assert_eq!(*with_locale.lock(), Some(Some("en-US".into())));

        let base_timing = parking_lot::Mutex::new(None);
        assert_eq!(*base_timing.lock(), None);
        publish_speech_timing_authority(&base_timing, None);
        assert_eq!(*base_timing.lock(), Some(None));
    }

    #[test]
    fn native_epoch_conversion_accepts_boundary_and_rejects_overflow() {
        assert_eq!(checked_epoch_ms(0), Ok(0));
        assert_eq!(checked_epoch_ms(u128::from(u64::MAX)), Ok(u64::MAX));
        assert!(checked_epoch_ms(u128::from(u64::MAX) + 1).is_err());
    }

    #[test]
    fn native_clock_returns_a_real_post_epoch_timestamp() {
        assert!(super::current_epoch_ms().expect("native system clock") > 0);
    }

    #[test]
    fn native_gameplay_rejects_late_content_and_opening_messages() {
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::ContentOffer { offer: offer() },
            )
            .unwrap_err()
            .contains("invalid native session message")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::ContentChunk {
                    full_mod_sha256: [1; 32],
                    offset: 0,
                    total_bytes: 1,
                    bytes: vec![0],
                },
            )
            .is_err()
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Reject {
                    reason: "session revoked".into(),
                },
            )
            .unwrap_err()
            .contains("session revoked")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Welcome {
                    your_seat: PlayerId(1),
                    mission_id: "late".into(),
                    mission_seed: 1,
                    sim_config: robin_engine::engine::SimConfig::default(),
                    speech_timing_locale: None,
                    host_nickname: "host".into(),
                    session_id: robin_engine::multiplayer::MultiplayerSessionId([2; 32]),
                },
            )
            .unwrap_err()
            .contains("invalid native session message")
        );
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &cosign_state,
                None,
                super::NetMsg::Note("legal".into()),
            )
            .is_ok()
        );
        assert!(matches!(
            incoming_rx.recv().unwrap(),
            super::NetEvent::Note(note) if note == "legal"
        ));
    }

    #[test]
    fn native_gameplay_rejects_host_only_and_late_content_outbound() {
        assert!(
            client_gameplay_wire_msg(super::NetOutbound::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .unwrap_err()
            .contains("host-only")
        );
        assert!(
            client_gameplay_wire_msg(super::NetOutbound::ContentReady {
                full_mod_sha256: [1; 32],
            })
            .unwrap_err()
            .contains("after gameplay began")
        );
        assert!(matches!(
            client_gameplay_wire_msg(super::NetOutbound::ReadyToSim { frame: 7 }).unwrap(),
            super::NetMsg::ReadyToSim { frame: 7 }
        ));
    }

    #[test]
    fn native_server_gameplay_rejects_wrong_direction_messages() {
        assert!(validate_server_gameplay_wire_msg(&super::NetMsg::Note("legal".into())).is_ok());
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::ContentRequest {
                full_mod_sha256: [1; 32],
                resume_offset: 0,
            })
            .unwrap_err()
            .contains("ordinary peer session")
        );
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .unwrap_err()
            .contains("invalid server-session message")
        );
        assert!(
            validate_server_gameplay_wire_msg(&super::NetMsg::Hello {
                protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
                nickname: "late".into(),
                browser_auth: None,
                ranked_public_key: None,
            })
            .unwrap_err()
            .contains("invalid server-session message")
        );

        assert!(
            validate_server_gameplay_outbound(&super::NetOutbound::StateHash {
                frame: 1,
                hash: Some(2),
                clock_frame: Some(3),
                ms_until_next_frame: Some(4),
            })
            .is_ok()
        );
        assert!(
            validate_server_gameplay_outbound(&super::NetOutbound::ContentPrepared {
                full_mod_sha256: [1; 32],
            })
            .unwrap_err()
            .contains("client-only")
        );
    }

    #[test]
    fn peer_inputs_reject_host_authoritative_commands_before_broadcast() {
        let error = validate_peer_command_authority(
            PlayerId(2),
            &robin_engine::player_command::PlayerCommand::ConnectSeat {
                player_id: PlayerId(7),
                nickname: "forged".to_string(),
            },
        )
        .expect_err("a peer must not author transport seat lifecycle");
        assert!(error.contains("host-authoritative"));

        validate_peer_command_authority(
            PlayerId(2),
            &robin_engine::player_command::PlayerCommand::CrouchDown,
        )
        .expect("ordinary seat input remains admissible");
    }

    #[test]
    fn campaign_owners_isolate_transport_identity_and_handoffs() {
        let a = super::MultiplayerCampaignSession::default();
        let b = super::MultiplayerCampaignSession::default();
        assert_eq!(a.state().client_key.public(), a.state().client_key.public());
        assert_ne!(a.state().client_key.public(), b.state().client_key.public());
        let continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 2,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(a.state(), continuation.clone());
        super::publish_host_session_continuation(b.state(), continuation.clone());
        a.discard_host_continuation().unwrap();
        drop(a);
        assert!(
            super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 3)
                .is_err()
        );
        assert!(
            super::pending_host_session_continuation(
                b.state(),
                iroh::SecretKey::generate().public(),
                2
            )
            .is_err()
        );
        let restored =
            super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 2)
                .unwrap()
                .unwrap();
        assert_eq!(restored.owner_seats, continuation.owner_seats);
        assert_eq!(restored.session_id, continuation.session_id);
        // Reading for replacement preparation is transactional: failure before
        // successful endpoint publication leaves the handoff intact.
        assert!(b.state().continuation.lock().is_some());
    }

    #[test]
    fn failed_startup_cancellation_joins_bridge_with_sender_still_alive() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (bridge, _async_receiver) = super::spawn_outgoing_bridge(
            "test-campaign-failed-startup",
            receiver,
            cancellation.clone(),
        )
        .unwrap();
        cancellation.store(true, std::sync::atomic::Ordering::Release);
        bridge.join().unwrap();
        drop(sender);
    }

    #[test]
    fn failed_campaign_server_start_keeps_handoff_and_releases_lease() {
        let campaign = super::MultiplayerCampaignSession::default();
        let key = iroh::SecretKey::from_bytes(&[3; 32]);
        let continuation = HostSessionContinuation {
            host_endpoint_id: key.public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 2,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        let (_channels, incoming, outgoing, cursor, snapshot) =
            crate::multiplayer::NetChannels::new();
        let result = super::start_server_inner(
            &campaign,
            key,
            "host".into(),
            "Dem_Lei_MP".into(),
            42,
            robin_engine::engine::SimConfig::default(),
            None,
            incoming,
            outgoing,
            cursor,
            snapshot,
            3,
            None,
            false,
        );
        let Err(error) = result else {
            panic!("mismatched replacement unexpectedly started");
        };
        assert!(error.to_string().contains("expects 2 players"));
        let pending = super::pending_host_session_continuation(
            campaign.state(),
            continuation.host_endpoint_id,
            2,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pending.owner_seats, continuation.owner_seats);
        assert!(campaign.reserve_server().is_ok());
    }

    #[test]
    fn campaign_server_lease_rejects_overlap_and_releases_failed_preparation() {
        let campaign = super::MultiplayerCampaignSession::default();
        let lease = campaign.reserve_server().unwrap();
        assert!(campaign.reserve_server().is_err());
        assert!(campaign.discard_host_continuation().is_err());
        let other = super::MultiplayerCampaignSession::default();
        let other_lease = other.reserve_server().unwrap();
        drop(lease);
        let replacement = campaign.reserve_server().unwrap();
        assert!(other.reserve_server().is_err());
        drop(other_lease);
        drop(replacement);
        assert!(campaign.reserve_server().is_ok());
    }

    #[test]
    fn campaign_serialization_cannot_restore_transport_authority() {
        let campaign = super::MultiplayerCampaignSession::default();
        let encoded = serde_json::to_string(&campaign).unwrap();
        assert_eq!(encoded, "{}");
        let decoded: super::MultiplayerCampaignSession = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.state.is_none());
    }

    #[test]
    fn campaign_repeated_publication_merges_authenticated_seats() {
        let campaign = super::MultiplayerCampaignSession::default();
        let mut continuation = HostSessionContinuation {
            host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
            session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
            expected_players: 3,
            owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
            relay_url: None,
        };
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        continuation.owner_seats =
            std::collections::HashMap::from([(PeerOwner::Browser([9; 32]), 2)]);
        super::publish_host_session_continuation(campaign.state(), continuation.clone());
        assert_eq!(
            super::pending_host_session_continuation(
                campaign.state(),
                continuation.host_endpoint_id,
                3
            )
            .unwrap()
            .unwrap()
            .owner_seats
            .len(),
            2
        );
    }

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
        let browser_claim = peers
            .claim_seat(browser, "renamed browser", ranked_identity(7), browser_tx)
            .unwrap();
        let (native_tx, _native_rx) = unbounded_channel();
        let native_claim = peers
            .claim_seat(native, "renamed native", ranked_identity(8), native_tx)
            .unwrap();

        assert_eq!(browser_claim.seat, 1);
        assert_eq!(browser_claim.kind, SeatClaimKind::Reconnect);
        assert_eq!(native_claim.seat, 2);
        assert_eq!(native_claim.kind, SeatClaimKind::Reconnect);
        let (intruder_tx, _intruder_rx) = unbounded_channel();
        assert!(
            peers
                .claim_seat(
                    PeerOwner::Browser([9; 32]),
                    "same nickname",
                    ranked_identity(9),
                    intruder_tx,
                )
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
    fn server_cosign_state_targets_one_authenticated_seat_and_rejects_duplicates() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 70);
        let key = iroh::SecretKey::generate();
        let response = signed_response(&request, &key);
        let mut peers = ServerPeers::new(3);
        let (seat_one_tx, mut seat_one_rx) = unbounded_channel();
        let (seat_two_tx, mut seat_two_rx) = unbounded_channel();
        peers.senders.insert(1, seat_one_tx);
        peers.senders.insert(2, seat_two_tx);
        admit_ranked_test_identity(&mut peers, 1, &key);

        let target = peers
            .begin_leaderboard_cosign(PlayerId(1), request)
            .unwrap();
        target
            .send(robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(
                request,
            ))
            .unwrap();
        assert!(matches!(
            seat_one_rx.try_recv().unwrap(),
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(decoded)
                if decoded == request
        ));
        assert!(
            seat_two_rx.try_recv().is_err(),
            "request must not broadcast"
        );

        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(2), &response)
                .unwrap_err()
                .contains("wrong-target")
        );
        peers
            .complete_leaderboard_cosign(PlayerId(1), &response)
            .unwrap();
        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(1), &response)
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            peers
                .begin_leaderboard_cosign(PlayerId(1), request)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn server_cosign_state_allows_same_final_request_for_distinct_targets_only() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 71);
        let mut peers = ServerPeers::new(3);
        for seat in [1, 2] {
            let (sender, _receiver) = unbounded_channel();
            peers.senders.insert(seat, sender);
            peers
                .begin_leaderboard_cosign(PlayerId(seat), request)
                .unwrap();
        }
        assert_eq!(peers.leaderboard_cosign.len(), 2);
        assert_eq!(peers.leaderboard_cosign_seen.len(), 2);
    }

    #[test]
    fn invalid_cosign_signature_does_not_consume_the_pending_request() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::CampaignContinuation, 72);
        let mut peers = ServerPeers::new(2);
        let admitted_key = iroh::SecretKey::generate();
        let (sender, _receiver) = unbounded_channel();
        peers.senders.insert(1, sender);
        admit_ranked_test_identity(&mut peers, 1, &admitted_key);
        peers
            .begin_leaderboard_cosign(PlayerId(1), request)
            .unwrap();

        let invalid = signed_response(&request, &iroh::SecretKey::generate());
        assert!(
            peers
                .complete_leaderboard_cosign(PlayerId(1), &invalid)
                .unwrap_err()
                .contains("other than its admitted durable identity")
        );
        assert_eq!(peers.leaderboard_cosign.len(), 1);
        let valid = signed_response(&request, &admitted_key);
        peers
            .complete_leaderboard_cosign(PlayerId(1), &valid)
            .unwrap();
        assert!(peers.leaderboard_cosign.is_empty());
    }

    #[test]
    fn client_wire_handler_never_exposes_unarmed_or_wrong_direction_cosign() {
        let request = leaderboard_request(LeaderboardCoSignPurposeV1::Submission, 73);
        let state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let (incoming_tx, incoming_rx) = std::sync::mpsc::channel();
        handle_client_wire_msg(
            &incoming_tx,
            &state,
            None,
            robin_engine::multiplayer::NetMsg::LeaderboardCoSignRequest(request),
        )
        .unwrap();
        assert!(incoming_rx.try_recv().is_err());
        assert_eq!(state.arm_request(request).unwrap(), Some(request));

        let response = signed_response(&request, &iroh::SecretKey::generate());
        assert!(
            handle_client_wire_msg(
                &incoming_tx,
                &state,
                None,
                robin_engine::multiplayer::NetMsg::LeaderboardCoSignResponse(response),
            )
            .unwrap_err()
            .contains("client-only")
        );
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
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let error = handle_client_wire_msg(
            &incoming_tx,
            &cosign_state,
            None,
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
        let cosign_state: SharedClientLeaderboardCoSignState = Arc::new(Default::default());
        let id = robin_engine::multiplayer::SnapshotTransitionId {
            session_id: robin_engine::multiplayer::MultiplayerSessionId([5; 32]),
            sequence: 9,
        };
        let save_bytes = vec![0, 17, 34, 255];
        handle_client_wire_msg(
            &incoming_tx,
            &cosign_state,
            None,
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
            &cosign_state,
            None,
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
        let first = peers
            .claim_seat(owner, "Robin", ranked_identity(7), first_tx)
            .unwrap();
        let seat = first.seat;
        let generation = first.generation;
        assert_eq!(seat, 1);
        assert_eq!(first.kind, SeatClaimKind::Fresh);
        assert!(peers.sim_connected_seats.insert(seat));

        let (replacement_tx, _replacement_rx) = unbounded_channel();
        let replacement = peers
            .claim_seat(owner, "Robin renamed", ranked_identity(7), replacement_tx)
            .unwrap();
        let replacement_seat = replacement.seat;
        let replacement_generation = replacement.generation;
        assert_eq!(replacement_seat, seat);
        assert_eq!(replacement.kind, SeatClaimKind::ActiveReplacement);
        assert!(peers.sim_connected_seats.contains(&seat));
        assert_ne!(replacement_generation, generation);
        assert_eq!(peers.release_seat_if_owner(seat, owner, generation), None);

        assert_eq!(
            peers.release_seat_if_owner(seat, owner, replacement_generation),
            Some(true)
        );
        let (other_tx, _other_rx) = unbounded_channel();
        let other_claim = peers
            .claim_seat(other, "Robin renamed", ranked_identity(8), other_tx)
            .unwrap();
        let other_seat = other_claim.seat;
        assert_eq!(
            other_seat, 2,
            "a matching nickname grants no seat authority"
        );

        let (rejoin_tx, _rejoin_rx) = unbounded_channel();
        let rejoined = peers
            .claim_seat(owner, "New name", ranked_identity(7), rejoin_tx)
            .unwrap();
        let rejoined_seat = rejoined.seat;
        assert_eq!(rejoined_seat, seat);
        assert_eq!(rejoined.kind, SeatClaimKind::Reconnect);
    }

    #[test]
    fn real_iroh_ranked_admission_uses_durable_key_and_gates_begin_and_reconnect() {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::INFO)
            .try_init();
        let host_key = iroh::SecretKey::generate();
        let setup = official_ranked_setup(&host_key);
        let transport_key = iroh::SecretKey::generate();
        let durable_key = iroh::SecretKey::generate();
        let transport_public = *transport_key.public().as_bytes();
        let durable_public = *durable_key.public().as_bytes();
        assert_ne!(transport_public, durable_public);

        let (server_in_tx, server_in_rx) = channel();
        let (server_out_tx, server_out_rx) = channel();
        let mut server = start_server_with_key(
            host_key,
            "host".into(),
            "Dem_Lei_MP".into(),
            7,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            Arc::new(AtomicU32::new(0)),
            Arc::new(StdMutex::new(None)),
            2,
        )
        .expect("start real iroh ranked host");
        server
            .install_ranked_session_setup(Some(setup.clone()))
            .expect("install ranked host setup");

        let (client_in_tx, client_in_rx) = channel();
        let (client_out_tx, client_out_rx) = channel();
        let mut client = connect_client_with_keys(
            transport_key.clone(),
            Some(durable_key.clone()),
            server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect ranked client");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });

        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        let setup_deadline = Instant::now() + Duration::from_millis(200);
        loop {
            match client_in_rx
                .recv_timeout(setup_deadline.saturating_duration_since(Instant::now()))
            {
                Ok(NetEvent::BeginSim { .. }) => {
                    panic!("ranked admission must resolve before BeginSim")
                }
                Ok(NetEvent::RankedBrowseOnly { reason }) => {
                    panic!("ReadyToSim resolved pending ranked setup as browse-only: {reason:?}")
                }
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("ranked client event channel closed before setup")
                }
            }
        }
        assert!(
            super::ranked_lifecycle_lock(&client.ranked_lifecycle())
                .browse_only_reason()
                .is_none(),
            "ReadyToSim must leave explicit ranked setup unresolved"
        );

        client
            .install_ranked_session_setup(Some(setup.clone()))
            .expect("install exact ranked client setup");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedJoinAccepted(_))
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::BeginSim { .. })
        });
        recv_matching(&server_in_rx, Duration::from_secs(10), |event| {
            matches!(
                event,
                NetEvent::Input { input, .. }
                    if matches!(
                        input.command,
                        robin_engine::player_command::PlayerCommand::ConnectSeat {
                            player_id: PlayerId(1),
                            ..
                        }
                    )
            )
        });

        let submission_accepted = SubmissionAcceptedV1 {
            schema_version: SCHEMA_VERSION_V1,
            submission_id: OpaqueId::new("native-ranked-submission").unwrap(),
            state: SubmissionLifecycleV1::Queued,
            retry_after_ms: 250,
        };
        let accepted_document = robin_engine::multiplayer::RankedSubmissionAcceptedDocument::new(
            crate::leaderboard_ranked_session::encode_ranked_wire_document(&submission_accepted)
                .unwrap(),
        )
        .unwrap();
        server_out_tx
            .send(NetOutbound::RankedSubmissionAccepted {
                to: PlayerId(1),
                accepted: accepted_document,
            })
            .unwrap();
        let received = recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedSubmissionAccepted(_))
        });
        let NetEvent::RankedSubmissionAccepted(received) = received else {
            unreachable!()
        };
        let decoded: SubmissionAcceptedV1 =
            crate::leaderboard_ranked_session::decode_ranked_wire_document(received.as_bytes())
                .unwrap();
        assert_eq!(decoded, submission_accepted);

        {
            let ranked_lifecycle = server.ranked_lifecycle();
            let lifecycle = super::ranked_lifecycle_lock(&ranked_lifecycle);
            let session = lifecycle
                .ranked_session()
                .expect("host remains ranked after admission");
            let guest = session
                .participant_claims()
                .into_iter()
                .find(|participant| participant.seat == 1)
                .expect("guest claim retained");
            assert_eq!(*guest.public_key.as_bytes(), durable_public);
            assert_eq!(
                *guest
                    .join_attestation
                    .expect("guest claim is attested")
                    .claim
                    .transport_endpoint_id
                    .as_bytes(),
                transport_public
            );
        }

        let disconnected_sender = server
            .context
            .peers
            .lock()
            .senders
            .remove(&1)
            .expect("ranked client has an active server writer");
        drop(disconnected_sender);
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::Disconnected)
        });
        recv_matching(&server_in_rx, Duration::from_secs(10), |event| {
            matches!(
                event,
                NetEvent::Input { input, .. }
                    if matches!(
                        input.command,
                        robin_engine::player_command::PlayerCommand::DisconnectSeat {
                            player_id: PlayerId(1)
                        }
                    )
            )
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::Reconnected)
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });
        let reconnect_deadline = Instant::now() + Duration::from_secs(10);
        let mut accepted_before_begin = false;
        loop {
            let event = client_in_rx
                .recv_timeout(reconnect_deadline.saturating_duration_since(Instant::now()))
                .expect("ranked reconnect did not resume before timeout");
            match event {
                NetEvent::RankedJoinAccepted(_) => accepted_before_begin = true,
                NetEvent::BeginSim { .. } => {
                    assert!(
                        accepted_before_begin,
                        "cached BeginSim bypassed reconnect admission"
                    );
                    break;
                }
                NetEvent::Fatal(error) => panic!("ranked reconnect failed: {error}"),
                _ => {}
            }
        }
        client.shutdown();
        server.shutdown();
    }

    #[test]
    fn real_iroh_ready_before_browse_downgrade_still_begins_gameplay() {
        let (server_in_tx, _server_in_rx) = channel();
        let (server_out_tx, server_out_rx) = channel();
        let mut server = start_server_with_key(
            iroh::SecretKey::generate(),
            "host".into(),
            "Dem_Lei_MP".into(),
            7,
            robin_engine::engine::SimConfig::default(),
            Some("en-US".into()),
            server_in_tx,
            server_out_rx,
            Arc::new(AtomicU32::new(0)),
            Arc::new(StdMutex::new(None)),
            2,
        )
        .expect("start real iroh browse-only host");
        let (client_in_tx, client_in_rx) = channel();
        let (client_out_tx, client_out_rx) = channel();
        let mut client = connect_client_with_keys(
            iroh::SecretKey::generate(),
            None,
            server.connect_string(),
            "alice".into(),
            client_in_tx,
            client_out_rx,
        )
        .expect("connect browse-only client");
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::AssignedLocalSeat(PlayerId(1)))
        });
        client_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        server_out_tx
            .send(NetOutbound::ReadyToSim { frame: 0 })
            .unwrap();
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::RankedBrowseOnly { .. })
        });
        recv_matching(&client_in_rx, Duration::from_secs(10), |event| {
            matches!(event, NetEvent::BeginSim { .. })
        });
        assert!(
            super::ranked_lifecycle_lock(&server.ranked_lifecycle())
                .browse_only_reason()
                .is_some()
        );
        assert!(
            super::ranked_lifecycle_lock(&client.ranked_lifecycle())
                .browse_only_reason()
                .is_some()
        );
        client.shutdown();
        server.shutdown();
    }
}
