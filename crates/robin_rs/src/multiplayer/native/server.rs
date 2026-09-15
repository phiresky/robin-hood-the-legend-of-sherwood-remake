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
    ) -> Result<crate::multiplayer::join_ticket::BrowserJoinTicket, MultiplayerError> {
        crate::multiplayer::join_ticket::BrowserJoinTicket::issue(
            &self.host_key,
            &self.endpoint_addr,
            self.session_id.0,
            try_current_epoch_ms()? / 1000,
            crate::multiplayer::join_ticket::BrowserJoinTicketContent {
                content_edition,
                content_identity_sha256,
                mission_id: self.mission_id.clone(),
                mission_profile_id,
                expected_players,
            },
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum InactivePeerSession {
    Released,
    Superseded { current_generation: u64 },
    Detached,
}

/// Not serde: `Protocol` carries the typed transport error.
#[derive(Clone, Debug)]
pub(super) enum PeerDispatchFailure {
    Inactive {
        seat: PlayerId,
        generation: u64,
        kind: InactivePeerSession,
    },
    Protocol(MultiplayerError),
}

impl std::error::Error for PeerDispatchFailure {}

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
            Self::Protocol(error) => error.fmt(f),
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
) -> Result<Option<PeerReaderExit>, MultiplayerError> {
    match result {
        Ok(()) => Ok(None),
        Err(error @ PeerDispatchFailure::Inactive { .. }) => {
            tracing::debug!(%error, "peer reader lost authority; draining its existing writer");
            Ok(Some(PeerReaderExit::Inactive))
        }
        Err(PeerDispatchFailure::Protocol(error)) => Err(error),
    }
}

impl ServerPeers {
    pub(super) fn new(expected_players: u32) -> Self {
        Self {
            sessions: PeerSessions::new(expected_players),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
        }
    }

    pub(super) fn from_continuation(continuation: &HostSessionContinuation) -> Self {
        Self {
            sessions: PeerSessions::from_continuation(continuation),
            readiness: ReadyBarrier::default(),
            transitions: SnapshotTransitions::default(),
        }
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
) -> Result<Option<(u32, u64, Vec<UnboundedSender<NetMsg>>)>, MultiplayerError> {
    let Some(begin_frame) = peers.readiness.candidate(
        peers.sessions.expected_players(),
        peers.sessions.readiness(),
    ) else {
        return Ok(None);
    };
    let start_epoch_ms = try_current_epoch_ms()?.checked_add(500).ok_or_else(|| {
        MultiplayerError::LocalState(
            "multiplayer BeginSim timestamp exceeds the u64 Unix range".into(),
        )
    })?;
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
    pub(super) continued_session: bool,
    pub(super) relay_url: Mutex<Option<iroh::RelayUrl>>,
    pub(super) speech_timing_locale: Option<String>,
    pub(super) frame_cursor: FrameCursor,
    pub(super) initial_snapshot: InitialSnapshot,
    pub(super) content: Option<HostedModContent>,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
}

pub(super) fn fail_server(context: &ServerContext, error: MultiplayerError) {
    if !context.cancellation.swap(true, Ordering::AcqRel) {
        let _ = context
            .incoming_tx
            .send(NetEvent::Fatal(NetFatal::new(error)));
    }
    let _ = context.shutdown_tx.send(true);
}

/// Start with an explicit identity key and a throwaway campaign. Tests use
/// this to avoid touching the per-install on-disk identity; production hosting
/// uses [`start_server_in_campaign`].
#[cfg(test)]
pub(in crate::multiplayer) fn start_server_with_key(
    key: SecretKey,
    config: ServerConfig,
    channels: ServerChannels,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        &MultiplayerCampaignSession::default(),
        key,
        config,
        channels,
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

/// The transport-side ends of one mission's game-loop channels. The matching
/// ends stay in the [`NetChannels`](crate::multiplayer::NetChannels) created
/// alongside them by [`NetChannels::new_server`](crate::multiplayer::NetChannels::new_server).
///
/// Not serde: these are live channel ends and shared slots, not data.
pub struct ServerChannels {
    /// Events the server publishes to the game loop.
    pub incoming_tx: Sender<NetEvent>,
    /// Messages the game loop queues for broadcast.
    pub outgoing_rx: Receiver<NetOutbound>,
    /// The game loop's current simulation frame.
    pub frame_cursor: FrameCursor,
    /// Latest full snapshot handed to joining peers.
    pub initial_snapshot: InitialSnapshot,
}

impl crate::multiplayer::NetChannels {
    /// [`NetChannels::new`](Self::new) with the transport-side ends bundled
    /// for [`start_server_in_campaign`].
    pub fn new_server() -> (Self, ServerChannels) {
        let (channels, incoming_tx, outgoing_rx, frame_cursor, initial_snapshot) = Self::new();
        (
            channels,
            ServerChannels {
                incoming_tx,
                outgoing_rx,
                frame_cursor,
                initial_snapshot,
            },
        )
    }
}

/// Start a mission transport within an explicitly owned campaign.
pub fn start_server_in_campaign(
    campaign: &MultiplayerCampaignSession,
    config: ServerConfig,
    channels: ServerChannels,
    content: Option<HostedModContent>,
) -> std::io::Result<ServerHandle> {
    start_server_inner(
        campaign,
        game_secret_key().map_err(std::io::Error::other)?,
        config,
        channels,
        content,
    )
}

pub(super) fn start_server_inner(
    campaign: &MultiplayerCampaignSession,
    key: SecretKey,
    config: ServerConfig,
    channels: ServerChannels,
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
    // Unpacked before the first fallible step, so every channel end is moved
    // or dropped at exactly the points the former by-value parameters were.
    let ServerChannels {
        incoming_tx,
        outgoing_rx,
        frame_cursor,
        initial_snapshot,
    } = channels;
    let campaign_lease = campaign.reserve_server()?;
    robin_engine::multiplayer::validate_display_name(&host_nickname)
        .map_err(std::io::Error::other)?;
    robin_engine::multiplayer::validate_mission_id(&mission_id).map_err(std::io::Error::other)?;
    if !(1..=crate::multiplayer::MAX_MULTIPLAYER_PLAYERS).contains(&expected_players) {
        return Err(std::io::Error::other(format!(
            "multiplayer expected-player count must be between 1 and {}, got {expected_players}",
            crate::multiplayer::MAX_MULTIPLAYER_PLAYERS
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
        std::sync::mpsc::sync_channel::<Result<(EndpointId, EndpointAddr), MultiplayerError>>(1);
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
                    let _ =
                        startup_tx.send(Err(MultiplayerError::transport("build tokio runtime", e)));
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
            return Err(std::io::Error::other(MultiplayerError::transport(
                "server startup channel closed",
                e,
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
    startup_tx: std::sync::mpsc::SyncSender<Result<(EndpointId, EndpointAddr), MultiplayerError>>,
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
            let _ = startup_tx.send(Err(MultiplayerError::Unavailable(
                "iroh relay did not become reachable within 15 seconds; disable browser join-link publication for a native-only game"
                    .into(),
            )));
            return;
        }
        if endpoint.addr().relay_urls().next().is_none() {
            endpoint.close().await;
            let _ = startup_tx.send(Err(MultiplayerError::Unavailable(
                "iroh reported online without a relay URL; a browser invitation cannot be published"
                    .into(),
            )));
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
                let _ = context
                    .incoming_tx
                    .send(NetEvent::Fatal(NetFatal::new(error)));
            }
            Err(error) => {
                context.cancellation.store(true, Ordering::Release);
                let _ = context.incoming_tx.send(NetEvent::Fatal(NetFatal::new(
                    MultiplayerError::transport("multiplayer server outgoing pump failed", error),
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

pub(super) fn retry_begin_sim(context: &ServerContext) {
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

/// Release newly connected seats into simulation: replay the cached start
/// barrier to them, or try to release the barrier if it has not begun yet.
pub(super) fn finish_seat_connections(context: &ServerContext, seats: &[u8]) {
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
        retry_begin_sim(context);
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
        match try_current_epoch_ms()
            .map_err(MultiplayerError::from)
            .and_then(|now| {
                now.checked_add(100).ok_or_else(|| {
                    MultiplayerError::LocalState(
                        "multiplayer seat connection timestamp exceeds the u64 Unix range".into(),
                    )
                })
            }) {
            Ok(start_epoch_ms) => start_epoch_ms,
            Err(error) => {
                tracing::error!(%error, "seat connection could not produce a start time");
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
    finish_seat_connections(context, &connected_seats);
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
) -> Result<(), MultiplayerError> {
    let conn = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, incoming)
        .await
        .map_err(|_| MultiplayerError::Handshake("peer QUIC handshake timed out".into()))?
        .map_err(|e| MultiplayerError::transport("peer connecting", e))?;
    let remote_id = conn.remote_id();
    let peer_id = remote_id.to_string();
    tracing::info!(peer = %peer_id, "incoming connection");

    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_FRAME_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| {
            MultiplayerError::Handshake(
                "peer did not open a game stream before handshake timeout".into(),
            )
        })?
        .map_err(|e| MultiplayerError::transport("accept peer stream", e))?;

    // Receive Hello.  Reject anything else.
    let (nickname, browser_auth) = match read_frame_bounded_with_timeout(
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
        }) => {
            if protocol_version != NET_PROTOCOL_VERSION {
                let reason = format!(
                    "protocol mismatch (peer={protocol_version}, server={NET_PROTOCOL_VERSION})"
                );
                reject_opening(&mut send, &reason).await;
                return Err(MultiplayerError::Handshake(reason.into()));
            }
            (nickname, browser_auth)
        }
        Some(other) => {
            let reason = format!("expected Hello, got {other:?}");
            reject_opening(&mut send, &reason).await;
            return Err(MultiplayerError::Handshake(reason.into()));
        }
        None => {
            return Err(MultiplayerError::Handshake(
                "connection closed before Hello".into(),
            ));
        }
    };

    let owner = match authenticate_peer(&context, remote_id, browser_auth.as_ref()) {
        Ok(owner) => owner,
        Err(error) => {
            // The wire rejection carries the reason as text.
            reject_opening(&mut send, &error.to_string()).await;
            return Err(error);
        }
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

    let (seat_claim, mut write_rx) = match prepare_peer_session(&context, owner, &nickname) {
        Ok(prepared) => prepared,
        Err(error) => {
            reject_opening(&mut send, &error.to_string()).await;
            return Err(error);
        }
    };
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(seat_claim.seat);

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
            Ok::<(), MultiplayerError>(())
        };
        let reader = run_server_peer_reader(&context, assigned_seat, session_generation, &mut recv);
        drive_server_peer_io(reader, writer, TERMINAL_WRITER_DRAIN_TIMEOUT).await
    };

    release_peer_session(&context, assigned_seat, owner, session_generation);
    conn.close(CLOSE_GRACEFUL.into(), b"session over");

    result
}

/// Claim, opening queue publication and admission are one authority operation.
/// All network I/O happens later in the peer task's writer.
pub(super) fn prepare_peer_session(
    context: &ServerContext,
    owner: PeerOwner,
    nickname: &str,
) -> Result<(SeatClaim, UnboundedReceiver<NetMsg>), MultiplayerError> {
    let _authority = context.session_dispatch.lock();
    // Claim/reclaim a seat by authenticated owner, never by editable nickname.
    let seat_claim = {
        let mut p = context.peers.lock();
        let returning_seat = p.sessions.owner_seat(owner);
        if let Some(transition) = p.transitions.pending()
            && !returning_seat.is_some_and(|seat| transition.awaiting.contains(&seat))
        {
            return Err(MultiplayerError::Handshake(
                "host is changing missions; only pending participants may reconnect".into(),
            ));
        }
        let (write_tx, write_rx) = unbounded_channel::<NetMsg>();
        p.sessions
            .claim_seat(owner, nickname, write_tx)
            .map(|claim| (claim, write_rx))
    };
    let (seat_claim, write_rx) = seat_claim?;
    let assigned_seat_u8 = seat_claim.seat;
    let session_generation = seat_claim.generation;
    let assigned_seat = PlayerId(assigned_seat_u8);
    tracing::debug!(
        seat = assigned_seat_u8,
        generation = session_generation,
        kind = ?seat_claim.kind,
        "multiplayer peer claimed seat"
    );

    // Queue Welcome for this peer.  Goes through the writer queue so
    // the writer task is the only thing that touches the outbound
    // half of the stream.  If the host has cached an initial-state
    // snapshot we follow up with that — mid-mission joiners adopt it
    // instead of trying to reproduce engine init from seed alone.
    let writer_closed = |message: &'static str| MultiplayerError::ChannelClosed(message.into());
    let opening_result = (|| -> Result<(), MultiplayerError> {
        let p = context.peers.lock();
        {
            let sender = p.sessions.sender(&assigned_seat_u8).ok_or_else(|| {
                MultiplayerError::LocalState(
                    format!("claimed peer {assigned_seat:?} has no writer for Welcome").into(),
                )
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
                .map_err(|_| writer_closed("writer queue closed before Welcome"))?;
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
                    .map_err(|_| writer_closed("writer queue closed before InitialSnapshot"))?;
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
                    .map_err(|_| writer_closed("writer queue closed before transition Prepare"))?;
            }
            // TODO(mp-duplicate-cached-begin): a reconnecting (non-sim-connected)
            // seat also receives the cached BeginSim from
            // `connect_all_provisional_seats` below, so it sees it twice. Clients
            // tolerate the repeat; consider sending it from one place only.
            if let Some((frame, start_epoch_ms)) = p.readiness.begun {
                let begin_frame =
                    snapshot_frame.map_or(frame, |snapshot_frame| snapshot_frame.max(frame));
                let begin_start_epoch_ms = if begin_frame != frame {
                    try_current_epoch_ms()?.checked_add(100).ok_or_else(|| {
                        MultiplayerError::LocalState(
                            "multiplayer reconnect timestamp exceeds the u64 Unix range".into(),
                        )
                    })?
                } else {
                    start_epoch_ms
                };
                sender
                    .send(NetMsg::BeginSim {
                        frame: begin_frame,
                        start_epoch_ms: begin_start_epoch_ms,
                    })
                    .map_err(|_| writer_closed("writer queue closed before cached BeginSim"))?;
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

    // The stream is gameplay-compatible after Welcome: connect the new seat
    // into the deterministic simulation right away.
    connect_all_provisional_seats(context);
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
    // An obsolete reader's teardown must not affect a successor.
    let Some(was_sim_connected) = release else {
        return;
    };
    if was_sim_connected && !context.cancellation.load(Ordering::Acquire) {
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
}

pub(super) async fn admit_distributed_mod(
    send: &mut SendStream,
    recv: &mut RecvStream,
    content: &HostedModContent,
    host_endpoint_id: String,
) -> Result<bool, MultiplayerError> {
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
        Some(NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
            full_mod_sha256,
            resume_offset,
        })) if full_mod_sha256 == offer.full_mod_sha256 => resume_offset,
        Some(NetMsg::ContentRequest(robin_engine::multiplayer::ContentRequest {
            full_mod_sha256,
            ..
        })) => {
            return Err(MultiplayerError::ContentMismatch(
                format!(
                    "client requested distributed mod {}, offered {}",
                    robin_engine::spellforge::hex_hash(&full_mod_sha256),
                    robin_engine::spellforge::hex_hash(&offer.full_mod_sha256)
                )
                .into(),
            ));
        }
        Some(NetMsg::ContentReject(robin_engine::multiplayer::ContentReject {
            full_mod_sha256,
            reason,
        })) if full_mod_sha256 == offer.full_mod_sha256 => {
            return Err(MultiplayerError::ContentDeclined(
                format!("client declined exact host content: {reason}").into(),
            ));
        }
        Some(other) => {
            return Err(MultiplayerError::RemoteProtocol(
                format!("expected ContentRequest, got {other:?}").into(),
            ));
        }
        None => {
            return Err(MultiplayerError::Handshake(
                "connection closed before content decision".into(),
            ));
        }
    };
    if resume_offset > content.encoded.len() as u64 {
        return Err(MultiplayerError::ContentMismatch(
            format!(
                "client resume offset {resume_offset} exceeds content length {}",
                content.encoded.len()
            )
            .into(),
        ));
    }
    let mut offset = resume_offset as usize;
    let transfer_deadline = tokio::time::Instant::now() + CONTENT_DECISION_TIMEOUT;
    while offset < content.encoded.len() {
        let end = (offset + robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT)
            .min(content.encoded.len());
        let message = NetMsg::ContentChunk(robin_engine::multiplayer::ContentChunk {
            full_mod_sha256: offer.full_mod_sha256,
            offset: offset as u64,
            total_bytes: content.encoded.len() as u64,
            bytes: content.encoded[offset..end].to_vec(),
        });
        tokio::select! {
            result = write_frame_with_timeout(
                send,
                &message,
                CONTENT_TRANSFER_IDLE_TIMEOUT,
                "content chunk",
            ) => result?,
            _ = tokio::time::sleep_until(transfer_deadline) => {
                return Err(MultiplayerError::DeadlineExceeded {
                    phase: "content transfer".into(),
                    limit: CONTENT_DECISION_TIMEOUT,
                });
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
        Some(NetMsg::ContentReject(robin_engine::multiplayer::ContentReject {
            full_mod_sha256,
            reason,
        })) if full_mod_sha256 == offer.full_mod_sha256 => Err(MultiplayerError::ContentDeclined(
            format!("client rejected downloaded host content: {reason}").into(),
        )),
        Some(other) => Err(MultiplayerError::RemoteProtocol(
            format!("expected ContentReady/ContentPrepared, got {other:?}").into(),
        )),
        None => Err(MultiplayerError::Handshake(
            "connection closed before content readiness".into(),
        )),
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
) -> Result<PeerOwner, MultiplayerError> {
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
        return Err(MultiplayerError::Invitation(
            "browser invitation does not belong to this exact hosted mission session".into(),
        ));
    }
    if payload.mission_id != context.mission_id && !context.continued_session {
        return Err(MultiplayerError::Invitation(
            "browser invitation belongs to another hosted mission".into(),
        ));
    }
    let owner = PeerOwner::Browser(auth.durable_public_key);
    let use_kind = if context.peers.lock().sessions.owner_seat(owner).is_some() {
        crate::multiplayer::join_ticket::InvitationUse::RedeemedReconnect
    } else {
        crate::multiplayer::join_ticket::InvitationUse::Initial
    };
    ticket.validate_use_at(try_current_epoch_ms()? / 1000, use_kind)?;
    let public_key = iroh::PublicKey::from_bytes(&auth.durable_public_key).map_err(|error| {
        MultiplayerError::invalid_address("invalid durable browser public key", error)
    })?;
    let signature_bytes: [u8; iroh::Signature::LENGTH] =
        auth.signature.as_slice().try_into().map_err(|_| {
            MultiplayerError::Identity("browser seat proof signature must be 64 bytes".into())
        })?;
    let signature = iroh::Signature::from_bytes(&signature_bytes);
    let message = browser_seat_proof_message(
        context.session_id.0,
        *context.host_endpoint_id.as_bytes(),
        *remote_id.as_bytes(),
    );
    public_key.verify(&message, &signature).map_err(|_| {
        MultiplayerError::Identity(
            "browser seat proof does not bind this session and transport".into(),
        )
    })?;
    Ok(owner)
}

/// Keep the writer future pinned when the reader loses authority. Dropping and
/// restarting write_frame could duplicate a partially written frame header.
pub(super) async fn drive_server_peer_io(
    reader: impl std::future::Future<Output = Result<PeerReaderExit, MultiplayerError>>,
    writer: impl std::future::Future<Output = Result<(), MultiplayerError>>,
    drain_timeout: Duration,
) -> Result<(), MultiplayerError> {
    tokio::pin!(writer);
    tokio::select! {
        result = reader => match result? {
            PeerReaderExit::Closed => Ok(()),
            PeerReaderExit::Inactive => {
                tokio::time::timeout(drain_timeout, writer.as_mut())
                    .await
                    .map_err(|_| MultiplayerError::LocalState(
                        "inactive peer writer timed out draining terminal frames".into(),
                    ))?
                    .map_err(|error| error.context("peer writer"))
            }
        },
        result = writer.as_mut() => result.map_err(|error| error.context("peer writer")),
    }
}

pub(super) async fn run_server_peer_reader(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    recv: &mut RecvStream,
) -> Result<PeerReaderExit, MultiplayerError> {
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
    message: NetMsg,
) -> Result<(), PeerDispatchFailure> {
    let _authority = context.session_dispatch.lock();
    context
        .peers
        .lock()
        .sessions
        .authorize_session(seat, session_generation)?;
    apply_authenticated_peer_message(context, seat, session_generation, message)
        .map_err(PeerDispatchFailure::Protocol)
}

/// Called only while dispatch_server_peer_message retains the authority gate.
pub(super) fn apply_authenticated_peer_message(
    context: &ServerContext,
    seat: PlayerId,
    session_generation: u64,
    message: NetMsg,
) -> Result<(), MultiplayerError> {
    let remote = |message: String| MultiplayerError::RemoteProtocol(message.into());
    let closed = |message: &'static str| MultiplayerError::ChannelClosed(message.into());
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
        NetMsg::ModalProposal(proposal) => {
            if proposal.instance.session_id != context.session_id {
                return Err(remote(format!(
                    "peer {seat:?} submitted a modal proposal for another session"
                )));
            }
            context
                .incoming_tx
                .send(NetEvent::ModalProposal {
                    from: seat,
                    proposal,
                })
                .map_err(|_| closed("host modal proposal channel is closed"))?;
        }
        NetMsg::ModalDecision { .. } => {
            return Err(remote(format!(
                "peer {seat:?} attempted an authoritative modal decision"
            )));
        }
        NetMsg::ReadyToSim { frame } => {
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
                return Err(remote(format!(
                    "peer {seat:?} acknowledged a snapshot transition for another session"
                )));
            }
            let committed = {
                let mut peers = context.peers.lock();
                peers.transitions.acknowledge(seat, id)?;
                take_committed_snapshot_transition(&mut peers)
            };
            commit_snapshot_transition(context, committed);
        }
        NetMsg::PrepareSnapshotTransition { .. } | NetMsg::CommitSnapshotTransition { .. } => {
            return Err(remote(format!(
                "peer {seat:?} attempted a host-only snapshot transition message"
            )));
        }
        _ => unreachable!("server gameplay message was validated before dispatch"),
    }
    Ok(())
}

pub(super) fn validate_server_gameplay_wire_msg(message: &NetMsg) -> Result<(), MultiplayerError> {
    match message {
        NetMsg::Input { .. }
        | NetMsg::Note(_)
        | NetMsg::ModalProposal { .. }
        | NetMsg::ReadyToSim { .. }
        | NetMsg::SnapshotTransitionReady { .. } => Ok(()),
        NetMsg::ContentRequest { .. }
        | NetMsg::ContentReject { .. }
        | NetMsg::ContentReady { .. }
        | NetMsg::ContentPrepared { .. } => Err(MultiplayerError::RemoteProtocol(
            "content-admission message arrived in an ordinary peer session".into(),
        )),
        other => Err(MultiplayerError::RemoteProtocol(
            format!("client sent invalid server-session message {other:?}").into(),
        )),
    }
}

pub(super) fn validate_peer_command_authority(
    seat: PlayerId,
    command: &robin_engine::player_command::PlayerCommand,
) -> Result<(), MultiplayerError> {
    if command.requires_host_authority() {
        return Err(MultiplayerError::RemoteProtocol(
            format!("peer {seat:?} attempted host-authoritative command {command:?}").into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
