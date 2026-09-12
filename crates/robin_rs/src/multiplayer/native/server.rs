//! Native server transport lifecycle; shared framing stays in the parent.
use super::*;

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
    pub(super) session_id: MultiplayerSessionId,
    pub(super) endpoint_id: EndpointId,
    pub(super) endpoint_addr: EndpointAddr,
    pub(super) host_key: SecretKey,
    pub(super) mission_id: String,
    pub(super) ranked_lifecycle: SharedRankedSessionLifecycle,
    pub(super) context: Arc<ServerContext>,
    pub(super) preserve_on_shutdown: bool,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
    pub(super) runtime_thread: Option<JoinHandle<()>>,
    pub(super) bridge_thread: Option<JoinHandle<()>>,
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
            .sessions
            .ranked_identities()
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
        let max_concurrent_players = u16::try_from(peers.sessions.expected_players()).ok()?;
        let mut participant_public_keys = peers
            .sessions
            .ranked_identities()
            .map(|(_, identity)| identity)
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
        let _authority = self.context.session_dispatch.lock();
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
        content_edition: crate::multiplayer::join_ticket::BrowserContentEdition,
        content_identity_sha256: String,
        mission_profile_id: Option<u32>,
        expected_players: u32,
    ) -> Result<crate::multiplayer::join_ticket::BrowserJoinTicket, String> {
        crate::multiplayer::join_ticket::BrowserJoinTicket::issue(
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

    pub(in crate::multiplayer) fn preserve_session_for_next_mission(&mut self) {
        self.preserve_on_shutdown = true;
    }
}

pub(super) fn publish_context_continuation(context: &ServerContext) {
    let peers = context.peers.lock();
    let owner_seats = peers.sessions.owner_seats();
    publish_host_session_continuation(
        &context.campaign,
        HostSessionContinuation {
            host_endpoint_id: context.host_endpoint_id,
            session_id: context.session_id,
            expected_players: peers.sessions.expected_players(),
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
pub(super) struct ServerPeers {
    pub(super) sessions: PeerSessions,
    pub(super) readiness: ReadyBarrier,
    pub(super) transitions: SnapshotTransitions,
    pub(super) cosigns: CoSignTracker,
    pub(super) admission: RankedAdmissionTracker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum SeatClaimKind {
    Fresh,
    Reconnect,
    ActiveReplacement,
}

#[derive(Debug)]
pub(super) struct SeatClaim {
    pub(super) seat: u8,
    pub(super) generation: u64,
    pub(super) kind: SeatClaimKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct RankedPeerIdentity {
    pub(super) durable_public_key: Option<[u8; 32]>,
    pub(super) transport_endpoint_id: [u8; 32],
    pub(super) public_disclosure: ParticipantPublicDisclosureV1,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct PendingRankedAdmission {
    pub(super) seat: u8,
    pub(super) generation: u64,
    pub(super) kind: RankedAdmissionKind,
    pub(super) challenge: RankedJoinChallenge,
    // Restored diagnostics expire immediately; no serialized transport authority.
    #[serde(skip, default = "Instant::now")]
    pub(super) deadline: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum RankedAdmissionKind {
    Fresh,
    Reconnect,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum InactivePeerSession {
    Released,
    Superseded { current_generation: u64 },
    Detached,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum PeerDispatchFailure {
    Inactive {
        seat: PlayerId,
        generation: u64,
        kind: InactivePeerSession,
    },
    Protocol(String),
}

impl std::fmt::Display for PeerDispatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inactive {
                seat,
                generation,
                kind,
            } => {
                write!(f, "peer {seat:?} generation {generation} ")?;
                match kind {
                    InactivePeerSession::Released => {
                        f.write_str("has no active authenticated session")
                    }
                    InactivePeerSession::Superseded { current_generation } => {
                        write!(f, "was superseded by generation {current_generation}")
                    }
                    InactivePeerSession::Detached => f.write_str("has a detached writer"),
                }
            }
            Self::Protocol(detail) => f.write_str(detail),
        }
    }
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum PeerReaderExit {
    Closed,
    /// No new reader effects are authorized, but the old writer may own
    /// terminal frames queued before detachment, replacement, or release.
    Inactive,
}

pub(super) fn peer_reader_dispatch_result(
    result: Result<(), PeerDispatchFailure>,
) -> Result<Option<PeerReaderExit>, String> {
    match result {
        Ok(()) => Ok(None),
        Err(error @ PeerDispatchFailure::Inactive { .. }) => {
            tracing::debug!(%error, "peer reader lost authority; draining its existing writer");
            Ok(Some(PeerReaderExit::Inactive))
        }
        Err(error) => Err(error.to_string()),
    }
}

impl ServerPeers {
    pub(super) fn new(expected_players: u32) -> Self {
        Self {
            sessions: PeerSessions::new(expected_players),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
            cosigns: CoSignTracker::default(),
            admission: RankedAdmissionTracker::default(),
        }
    }

    pub(super) fn from_continuation(continuation: &HostSessionContinuation) -> Self {
        Self {
            sessions: PeerSessions::from_continuation(continuation),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
            cosigns: CoSignTracker::default(),
            admission: RankedAdmissionTracker::default(),
        }
    }

    /// Atomically retain the exact request before returning its one target's
    /// queue. This ordering ensures even an immediate peer response always
    /// finds pending authenticated server state.
    pub(super) fn begin_leaderboard_cosign(
        &mut self,
        target: PlayerId,
        request: LeaderboardCoSignRequestV1,
    ) -> Result<UnboundedSender<NetMsg>, String> {
        let sender = self.sessions.sender(&target.0).cloned().ok_or_else(|| {
            format!("leaderboard co-sign target {target:?} is not an authenticated active peer")
        })?;
        self.cosigns.begin(target, request)?;
        Ok(sender)
    }

    /// Verify and consume exactly one targeted request. The caller-supplied
    /// seat is stamped by the authenticated stream and never read from the
    /// response payload.
    pub(super) fn complete_leaderboard_cosign(
        &mut self,
        from: PlayerId,
        response: &LeaderboardCoSignResponse,
    ) -> Result<(), String> {
        let expected_signer = self
            .sessions
            .ranked_identity(&from.0)
            .and_then(|identity| identity.durable_public_key);
        self.cosigns.complete(from, expected_signer, response)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum PeerOwner {
    Native([u8; 32]),
    Browser([u8; 32]),
}

pub(super) fn take_committed_snapshot_transition(
    peers: &mut ServerPeers,
) -> Option<(
    robin_engine::multiplayer::SnapshotTransitionId,
    Vec<UnboundedSender<NetMsg>>,
)> {
    let id = peers.transitions.take_completed()?;
    let senders = peers.sessions.detach_all_writers();
    Some((id, senders))
}

pub(super) fn retain_transition_peer_for_reconnect(peers: &mut ServerPeers, seat: u8) {
    peers.transitions.retain_for_reconnect(seat);
}

pub(super) fn commit_snapshot_transition(
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

pub(super) fn maybe_begin_sim_locked(
    peers: &mut ServerPeers,
) -> Result<Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>, String> {
    let Some(begin_frame) = peers.readiness.candidate(
        peers.sessions.expected_players(),
        peers.sessions.readiness(),
    ) else {
        return Ok(None);
    };
    let start_epoch_ms = current_epoch_ms()?
        .checked_add(500)
        .ok_or_else(|| "multiplayer BeginSim timestamp exceeds the u64 Unix range".to_owned())?;
    let senders = peers
        .sessions
        .senders()
        .map(|(_, sender)| sender)
        .cloned()
        .collect();
    peers.readiness.commit(begin_frame, start_epoch_ms);
    Ok(Some((begin_frame, start_epoch_ms, senders)))
}

/// Per-session context shared by every server task.
pub(super) struct ServerContext {
    pub(super) _campaign_lease: CampaignServerLease,
    pub(super) campaign: Arc<CampaignTransportState>,
    // Outermost lock: each synchronous protocol event is atomic relative to
    // claim/release/writer detachment. Never hold it across an await.
    pub(super) session_dispatch: Mutex<()>,
    pub(super) peers: Mutex<ServerPeers>,
    pub(super) incoming_tx: Sender<NetEvent>,
    pub(super) host_nickname: String,
    pub(super) mission_id: String,
    pub(super) mission_seed: u64,
    pub(super) sim_config: robin_engine::engine::SimConfig,
    pub(super) host_endpoint_id: EndpointId,
    pub(super) session_id: MultiplayerSessionId,
    pub(super) ranked_lifecycle: SharedRankedSessionLifecycle,
    pub(super) ranked_browse_reason: Mutex<Option<RankedBrowseOnlyReason>>,
    pub(super) continued_session: bool,
    pub(super) relay_url: Mutex<Option<iroh::RelayUrl>>,
    pub(super) speech_timing_locale: Option<String>,
    pub(super) frame_cursor: FrameCursor,
    pub(super) initial_snapshot: InitialSnapshot,
    pub(super) content: Option<HostedModContent>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
}

pub(super) fn fail_server(context: &ServerContext, error: String) {
    if !context.cancellation.swap(true, Ordering::AcqRel) {
        let _ = context.incoming_tx.send(NetEvent::Fatal(error));
    }
    let _ = context.shutdown_tx.send(true);
}

/// Start with an explicit identity key. Tests use this to
/// avoid touching the per-install on-disk identity.
#[cfg(test)]
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
        ServerConfig {
            host_nickname: host_nickname,
            mission_id: mission_id,
            mission_seed: mission_seed,
            sim_config: sim_config,
            speech_timing_locale: speech_timing_locale,
            expected_players: expected_players,
            browser_join_enabled: false,
        },
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        None,
    )
}

/// Test-only explicit-key entry point for an exact hosted package. Browser
/// ticket publication is disabled; production hosting uses
/// [`start_server_in_campaign`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(in crate::multiplayer) fn start_server_with_key_and_content(
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
        ServerConfig {
            host_nickname: host_nickname,
            mission_id: mission_id,
            mission_seed: mission_seed,
            sim_config: sim_config,
            speech_timing_locale: None,
            expected_players: expected_players,
            browser_join_enabled: false,
        },
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        content,
    )
}

/// Immutable identity and admission policy for one hosted mission.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    pub host_nickname: String,
    pub mission_id: String,
    pub mission_seed: u64,
    pub sim_config: robin_engine::engine::SimConfig,
    pub speech_timing_locale: Option<String>,
    pub expected_players: u32,
    pub browser_join_enabled: bool,
}

/// Start a mission transport within an explicitly owned campaign.
pub fn start_server_in_campaign(
    campaign: &MultiplayerCampaignSession,
    config: ServerConfig,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        campaign,
        game_secret_key().map_err(std::io::Error::other)?,
        config,
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
        content,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_server_inner(
    campaign: &MultiplayerCampaignSession,
    key: SecretKey,
    config: ServerConfig,
    incoming_tx: Sender<NetEvent>,
    outgoing_rx: Receiver<NetOutbound>,
    frame_cursor: FrameCursor,
    initial_snapshot: InitialSnapshot,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    let ServerConfig {
        host_nickname,
        mission_id,
        mission_seed,
        sim_config,
        speech_timing_locale,
        expected_players,
        browser_join_enabled,
    } = config;
    let campaign_lease = campaign.reserve_server()?;
    robin_engine::multiplayer::validate_display_name(&host_nickname)
        .map_err(std::io::Error::other)?;
    robin_engine::multiplayer::validate_mission_id(&mission_id).map_err(std::io::Error::other)?;
    if !(1..=crate::multiplayer::join_ticket::MAX_MULTIPLAYER_PLAYERS).contains(&expected_players) {
        return Err(std::io::Error::other(format!(
            "multiplayer expected-player count must be between 1 and {}, got {expected_players}",
            crate::multiplayer::join_ticket::MAX_MULTIPLAYER_PLAYERS
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
        session_dispatch: Mutex::new(()),
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

pub(super) async fn run_server(
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
    if startup_tx
        .send(Ok((endpoint.id(), endpoint.addr())))
        .is_err()
    {
        // Startup's owner was dropped; there is nobody to own an accept loop.
        endpoint.close().await;
        return;
    }

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

pub(super) fn ranked_lifecycle_lock(
    lifecycle: &SharedRankedSessionLifecycle,
) -> std::sync::MutexGuard<'_, RankedSessionLifecycle> {
    lifecycle.lock().unwrap_or_else(|poisoned| {
        tracing::error!(
            "ranked session lifecycle lock was poisoned; retaining authoritative state"
        );
        poisoned.into_inner()
    })
}

pub(super) fn validate_official_ranked_session(
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

pub(super) fn publish_connect_seats(context: &ServerContext, seats: Vec<(u8, String)>) {
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

pub(super) fn retry_begin_sim_after_ranked_resolution(context: &ServerContext) {
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

pub(super) fn finish_ranked_seat_connections(context: &ServerContext, seats: &[u8]) {
    let cached_begin = {
        let peers = context.peers.lock();
        peers.readiness.begun.map(|(frame, start_epoch_ms)| {
            let senders = seats
                .iter()
                .filter_map(|seat| peers.sessions.sender(seat).cloned())
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
        server_dispatch::queue_cached_begin(&sender, begin_frame, begin_start_epoch_ms);
    }
}

pub(super) fn connect_all_provisional_seats(context: &ServerContext) {
    let seats = context.peers.lock().sessions.admit_provisional_sessions();
    let connected_seats = seats.iter().map(|(seat, _)| *seat).collect::<Vec<_>>();
    publish_connect_seats(context, seats);
    finish_ranked_seat_connections(context, &connected_seats);
}

pub(super) fn downgrade_ranked_session(
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
    context.peers.lock().admission.clear();
    if transitioned {
        *context.ranked_browse_reason.lock() = Some(wire_reason);
        tracing::warn!(reason = ?wire_reason, %detail, "ranked multiplayer downgraded; gameplay remains available");
        server_dispatch::broadcast_recoverable(
            context,
            NetMsg::RankedBrowseOnly {
                reason: wire_reason,
            },
        );
        if context
            .incoming_tx
            .send(NetEvent::RankedBrowseOnly {
                reason: wire_reason,
            })
            .is_err()
        {
            // The authoritative game loop is gone. Do not release provisional
            // peers into simulation after losing its admission notification.
            fail_server(
                context,
                "host event receiver closed during ranked downgrade".into(),
            );
            return;
        }
    }
    connect_all_provisional_seats(context);
}

pub(super) enum RankedAdmissionProgress {
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

pub(super) fn progress_ranked_admission(context: &ServerContext) {
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
                if let Some(pending) = peers.admission.pending()
                    && (peers.sessions.generation(&pending.seat) != Some(&pending.generation)
                        || peers.sessions.sender(&pending.seat).is_none())
                {
                    session.cancel_pending_join();
                    peers.admission.clear();
                }
                if peers.admission.pending().is_some() {
                    RankedAdmissionProgress::Idle
                } else {
                    let next_seat = peers
                        .sessions
                        .senders()
                        .map(|(seat, _)| seat)
                        .copied()
                        .filter(|seat| !peers.sessions.is_sim_connected(seat))
                        .min();
                    let Some(seat) = next_seat else {
                        return;
                    };
                    let identity = *peers.sessions.ranked_identity(&seat).unwrap_or_else(|| {
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
                                        let generation = *peers
                                            .sessions
                                            .generation(&seat)
                                            .expect("provisional seat has a session generation");
                                        let sender = peers
                                            .sessions
                                            .sender(&seat)
                                            .cloned()
                                            .expect("provisional seat has a sender");
                                        peers.admission.begin(PendingRankedAdmission {
                                            seat,
                                            generation,
                                            kind,
                                            challenge: challenge.clone(),
                                            deadline: Instant::now() + RANKED_ADMISSION_TIMEOUT,
                                        });
                                        RankedAdmissionProgress::Challenge { sender, challenge }
                                    }
                                    Err(detail) => {
                                        session.cancel_pending_join();
                                        RankedAdmissionProgress::Downgrade {
                                            reason: RankedBrowseOnlyReason::RankedProtocolViolation,
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
        RankedAdmissionProgress::Idle => (),
        RankedAdmissionProgress::BrowseOnly => {
            connect_all_provisional_seats(context);
        }
        RankedAdmissionProgress::Challenge { sender, challenge } => {
            if sender.send(NetMsg::RankedJoinChallenge(challenge)).is_err() {
                downgrade_ranked_session(
                    context,
                    RankedBrowseOnlyReason::RankedTransportInterrupted,
                    "ranked admission target disconnected before challenge delivery",
                );
            }
        }
        RankedAdmissionProgress::Downgrade { reason, detail } => {
            downgrade_ranked_session(context, reason, detail);
        }
    }
}

pub(super) fn resolve_ranked_before_ready(context: &ServerContext) {
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

pub(super) fn ranked_unavailable_browse_reason(
    reason: crate::multiplayer::RankedJoinUnavailableReason,
) -> RankedBrowseOnlyReason {
    match reason {
        crate::multiplayer::RankedJoinUnavailableReason::DurableIdentityUnavailable => {
            RankedBrowseOnlyReason::PeerIdentityUnavailable
        }
        crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionUnavailable
        | crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch => {
            RankedBrowseOnlyReason::PeerRankedSessionMismatch
        }
        crate::multiplayer::RankedJoinUnavailableReason::AttestationSigningFailed => {
            RankedBrowseOnlyReason::PeerAttestationRejected
        }
    }
}

pub(super) fn handle_ranked_join_response(
    context: &ServerContext,
    seat: PlayerId,
    generation: u64,
    identity: RankedPeerIdentity,
    response: RankedJoinResponse,
) {
    if context.peers.lock().sessions.generation(&seat.0) != Some(&generation) {
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
        let Some(pending) = peers.admission.pending().cloned() else {
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
                                .sessions
                                .sim_connected_seats()
                                .filter_map(|existing_seat| {
                                    peers.sessions.sender(existing_seat).cloned()
                                })
                                .collect::<Vec<_>>()
                        } else {
                            Vec::new()
                        };
                        peers.admission.clear();
                        assert!(
                            peers
                                .sessions
                                .admit_session(seat.0, generation)
                                .expect("ranked response retains authenticated dispatch authority"),
                            "ranked admission connected a seat already present in the simulation"
                        );
                        let nickname =
                            peers
                                .sessions
                                .nickname(&seat.0)
                                .cloned()
                                .unwrap_or_else(|| {
                                    panic!("admitted ranked seat {} has no nickname", seat.0)
                                });
                        let sender = peers.sessions.sender(&seat.0).cloned().unwrap_or_else(|| {
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

pub(super) async fn run_server_accept_loop(context: Arc<ServerContext>, endpoint: Endpoint) {
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

pub(super) async fn handle_incoming_peer(
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

    let (seat_claim, mut write_rx) =
        match prepare_peer_session(&context, owner, &nickname, ranked_identity) {
            Ok(prepared) => prepared,
            Err(reason) => {
                reject_opening(&mut send, &reason).await;
                return Err(reason);
            }
        };
    let assigned_seat_u8 = seat_claim.seat;
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(assigned_seat_u8);
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
        drive_server_peer_io(reader, writer, TERMINAL_WRITER_DRAIN_TIMEOUT).await
    };

    release_peer_session(&context, assigned_seat, owner, session_generation);
    conn.close(CLOSE_GRACEFUL.into(), b"session over");
    admission_monitor.abort();

    result
}

/// Claim, opening queue publication and admission are one authority operation.
/// All network I/O happens later in the peer task's writer.
pub(super) fn prepare_peer_session(
    context: &ServerContext,
    owner: PeerOwner,
    nickname: &str,
    ranked_identity: RankedPeerIdentity,
) -> Result<(SeatClaim, UnboundedReceiver<NetMsg>), String> {
    let _authority = context.session_dispatch.lock();
    // Claim/reclaim a seat by authenticated owner, never by editable nickname.
    let seat_claim = {
        let mut p = context.peers.lock();
        let returning_seat = p.sessions.owner_seat(owner);
        if let Some(transition) = p.transitions.pending()
            && !returning_seat.is_some_and(|seat| transition.awaiting.contains(&seat))
        {
            return Err(
                "host is changing missions; only pending participants may reconnect".to_string(),
            );
        }
        let (write_tx, write_rx) = unbounded_channel::<NetMsg>();
        p.sessions
            .claim_seat(owner, nickname, ranked_identity, write_tx)
            .map(|claim| (claim, write_rx))
    };
    let (seat_claim, write_rx) = seat_claim?;
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
        {
            let sender = p.sessions.sender(&assigned_seat_u8).ok_or_else(|| {
                format!("claimed peer {assigned_seat:?} has no writer for Welcome")
            })?;
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
                sender
                    .send(NetMsg::InitialSnapshot {
                        frame,
                        engine_bytes: bytes,
                    })
                    .map_err(|_| "writer queue closed before InitialSnapshot")?;
                Some(frame)
            } else {
                None
            };
            if let Some(transition) = p.transitions.pending() {
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
            if !ranked_admission_required && let Some((frame, start_epoch_ms)) = p.readiness.begun {
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
        p.sessions
            .release_seat_if_owner(assigned_seat_u8, owner, session_generation);
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
                context,
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
    progress_ranked_admission(context);
    Ok((seat_claim, write_rx))
}

pub(super) fn release_peer_session(
    context: &ServerContext,
    assigned_seat: PlayerId,
    owner: PeerOwner,
    session_generation: u64,
) {
    let _authority = context.session_dispatch.lock();
    let assigned_seat_u8 = assigned_seat.0;
    // On disconnect, park the authenticated owner identity so a future
    // reconnect reclaims the same deterministic seat. Nicknames are labels.
    let release = {
        let mut p = context.peers.lock();
        let release = p
            .sessions
            .release_seat_if_owner(assigned_seat_u8, owner, session_generation);
        if release.is_some() {
            retain_transition_peer_for_reconnect(&mut p, assigned_seat_u8);
        }
        release
    };
    // An obsolete reader's teardown must not progress/downgrade a successor.
    let Some(release) = release else {
        return;
    };
    if release && !context.cancellation.load(Ordering::Acquire) {
        let observation = {
            let mut lifecycle = ranked_lifecycle_lock(&context.ranked_lifecycle);
            lifecycle
                .ranked_mut()
                .map(|session| session.observe_disconnect(u16::from(assigned_seat_u8)))
        };
        if let Some(Err(error)) = observation {
            downgrade_ranked_session(
                context,
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
        broadcast_input(context, now, now, target, inp);
    } else if !release {
        let cancelled_pending = {
            let mut peers = context.peers.lock();
            peers
                .admission
                .cancel_for(assigned_seat_u8, session_generation)
        };
        if cancelled_pending
            && let Some(session) = ranked_lifecycle_lock(&context.ranked_lifecycle).ranked_mut()
        {
            session.cancel_pending_join();
        }
    }
    progress_ranked_admission(context);
}

pub(super) async fn monitor_ranked_admission(
    context: Arc<ServerContext>,
    seat: u8,
    generation: u64,
) {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let _authority = context.session_dispatch.lock();
        let status = {
            let peers = context.peers.lock();
            peers.admission.deadline(
                seat,
                generation,
                peers
                    .sessions
                    .sender(&seat)
                    .and_then(|_| peers.sessions.generation(&seat).copied()),
                peers.sessions.is_sim_connected(&seat),
                Instant::now(),
            )
        };
        match status {
            AdmissionDeadline::Finished => return,
            AdmissionDeadline::Waiting => {}
            AdmissionDeadline::Expired => {
                downgrade_ranked_session(
                    &context,
                    RankedBrowseOnlyReason::RankedTransportInterrupted,
                    format!("seat {seat} timed out during ranked admission"),
                );
                return;
            }
        }
    }
}

pub(super) async fn admit_distributed_mod(
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

pub(super) async fn reject_opening(send: &mut SendStream, reason: &str) {
    let reason = robin_engine::multiplayer::bounded_safe_diagnostic(
        reason,
        robin_engine::multiplayer::MAX_REJECT_REASON_BYTES,
    );
    if let Err(error) = write_frame(send, &NetMsg::Reject { reason }).await {
        tracing::debug!(%error, "failed to send multiplayer opening rejection");
    }
}

pub(super) fn authenticate_peer(
    context: &ServerContext,
    remote_id: EndpointId,
    browser_auth: Option<&BrowserPeerAuth>,
) -> Result<PeerOwner, String> {
    let Some(auth) = browser_auth else {
        return Ok(PeerOwner::Native(*remote_id.as_bytes()));
    };
    let ticket =
        crate::multiplayer::join_ticket::BrowserJoinTicket::decode_authenticated(&auth.join_code)?;
    let payload = ticket.payload();
    if payload.host_endpoint_id != context.host_endpoint_id.to_string()
        || ticket.session_id()? != context.session_id.0
        || payload.expected_players != context.peers.lock().sessions.expected_players()
    {
        return Err(
            "browser invitation does not belong to this exact hosted mission session".to_string(),
        );
    }
    if payload.mission_id != context.mission_id && !context.continued_session {
        return Err("browser invitation belongs to another hosted mission".to_string());
    }
    let owner = PeerOwner::Browser(auth.durable_public_key);
    let use_kind = if context.peers.lock().sessions.owner_seat(owner).is_some() {
        crate::multiplayer::join_ticket::InvitationUse::RedeemedReconnect
    } else {
        crate::multiplayer::join_ticket::InvitationUse::Initial
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

/// Keep the writer future pinned when the reader loses authority. Dropping and
/// restarting write_frame could duplicate a partially written frame header.
pub(super) async fn drive_server_peer_io(
    reader: impl std::future::Future<Output = Result<PeerReaderExit, String>>,
    writer: impl std::future::Future<Output = Result<(), String>>,
    drain_timeout: Duration,
) -> Result<(), String> {
    tokio::pin!(writer);
    tokio::select! {
        result = reader => match result? {
            PeerReaderExit::Closed => Ok(()),
            PeerReaderExit::Inactive => {
                tokio::time::timeout(drain_timeout, writer.as_mut())
                    .await
                    .map_err(|_| "inactive peer writer timed out draining terminal frames".to_string())?
                    .map_err(|error| format!("peer writer: {error}"))
            }
        },
        result = writer.as_mut() => result.map_err(|error| format!("peer writer: {error}")),
    }
}

pub(super) async fn run_server_peer_reader(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    ranked_identity: RankedPeerIdentity,
    recv: &mut RecvStream,
) -> Result<PeerReaderExit, String> {
    loop {
        let message = match read_frame(recv, InboundFramePolicy::ClientToServer).await {
            Ok(Some(message)) => message,
            ended => {
                // A half-close/read error must not cancel a terminal write
                // already queued by the host while this read was suspended.
                let _authority = context.session_dispatch.lock();
                if let Some(exit) = peer_reader_dispatch_result(
                    context
                        .peers
                        .lock()
                        .sessions
                        .authorize_session(seat, session_generation),
                )? {
                    return Ok(exit);
                }
                return ended.map(|_| PeerReaderExit::Closed);
            }
        };
        if let Some(exit) = peer_reader_dispatch_result(dispatch_server_peer_message(
            context,
            seat,
            session_generation,
            ranked_identity,
            message,
        ))? {
            return Ok(exit);
        }
    }
}

/// Execute one decoded peer event without yielding. The authority gate remains
/// held through validation, state transitions, and local/writer queue effects;
/// a superseding claim cannot slip between a generation check and its effect.
pub(super) fn dispatch_server_peer_message(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    ranked_identity: RankedPeerIdentity,
    message: NetMsg,
) -> Result<(), PeerDispatchFailure> {
    let _authority = context.session_dispatch.lock();
    context
        .peers
        .lock()
        .sessions
        .authorize_session(seat, session_generation)?;
    apply_authenticated_peer_message(context, seat, session_generation, ranked_identity, message)
        .map_err(PeerDispatchFailure::Protocol)
}

/// Called only while dispatch_server_peer_message retains the authority gate.
pub(super) fn apply_authenticated_peer_message(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    ranked_identity: RankedPeerIdentity,
    message: NetMsg,
) -> Result<(), String> {
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
                p.sessions.record_ready(seat.0, session_generation, frame)?;
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
                peers.transitions.acknowledge(seat, id)?;
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
                return Ok(());
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
                .map_err(|_| "host leaderboard co-sign response channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationReceiptSelection(selection) => {
            let decoded = decode_ranked_wire_document::<
                CampaignContinuationReceiptSelectionResponseV1,
            >(selection.as_bytes())
            .map_err(|error| format!("invalid continuation receipt selection: {error}"))?;
            let expected_key = context
                    .peers
                    .lock()
                    .sessions.ranked_identity(&seat.0)
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
                .map_err(|_| "host continuation receipt selection channel is closed".to_string())?;
        }
        NetMsg::RankedContinuationPreflightSignature(signature) => {
            let signature_document =
                crate::leaderboard_ranked_session::decode_canonical_ranked_wire_document::<
                    ParticipantSignatureV1,
                >(signature.as_bytes())
                .map_err(|error| format!("invalid continuation preflight signature: {error}"))?;
            if signature_document.public_key.is_zero() || signature_document.signature.is_zero() {
                return Err(
                    "continuation preflight signature contains zero key material".to_string(),
                );
            }
            let expected_key = context
                    .peers
                    .lock()
                    .sessions.ranked_identity(&seat.0)
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
    Ok(())
}

pub(super) fn validate_server_gameplay_wire_msg(message: &NetMsg) -> Result<(), String> {
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

pub(super) fn validate_peer_command_authority(
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
