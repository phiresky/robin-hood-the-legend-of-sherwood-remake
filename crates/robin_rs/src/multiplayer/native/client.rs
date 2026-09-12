//! Native client transport lifecycle; shared framing stays in the parent.
use super::*;

// ─── Client ──────────────────────────────────────────────────────

/// Handle to an active client connection.
pub struct ClientHandle {
    pub(super) session_metadata: Arc<Mutex<Option<ClientSessionMetadata>>>,
    /// Present when the host requires content admission before Welcome. The
    /// game/menu must explicitly trust, download, validate, mount, and answer
    /// this exact offer; the transport never silently approves it.
    pub(super) content_offer: Arc<Mutex<Option<robin_engine::multiplayer::DistributedModOffer>>>,
    pub(super) ranked_lifecycle: SharedRankedSessionLifecycle,
    pub(super) ranked_setup_tx:
        UnboundedSender<Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>>,
    pub(super) ranked_setup_sent: AtomicBool,
    pub(super) ranked_local_public_key: Option<PublicKey32>,
    pub(super) ranked_authenticated_host_public_key: PublicKey32,
    pub(super) cancellation: Arc<AtomicBool>,
    pub(super) io_thread: Option<JoinHandle<()>>,
}

impl ClientHandle {
    pub fn session_metadata(&self) -> Option<ClientSessionMetadata> {
        self.session_metadata.lock().clone()
    }

    pub fn session_id(&self) -> Option<MultiplayerSessionId> {
        self.session_metadata().map(|session| session.session_id)
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
        self.session_metadata().map(|session| session.seat)
    }

    pub fn mission_seed(&self) -> Option<u64> {
        self.session_metadata().map(|session| session.mission_seed)
    }

    pub fn mission_sim_config(&self) -> Option<robin_engine::engine::SimConfig> {
        self.session_metadata().map(|session| session.sim_config)
    }

    pub fn mission_id(&self) -> Option<String> {
        self.session_metadata().map(|session| session.mission_id)
    }

    pub fn speech_timing_locale(&self) -> Option<String> {
        self.session_metadata()
            .and_then(|session| session.speech_timing_locale)
    }

    /// The outer option distinguishes a pending handshake from an explicit
    /// `None`, which authoritatively selects base `Data/Sounds` timing.
    pub fn speech_timing_authority(&self) -> Option<Option<String>> {
        self.session_metadata()
            .map(|session| session.speech_timing_locale)
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

/// Explicit-key client entry used by transport tests.
/// The injected key owns both the test transport and its ranked identity;
/// production still derives only the durable ranked key from install state.
#[cfg(test)]
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

pub(super) fn connect_client_inner(
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
    let session_metadata = Arc::new(Mutex::new(None));
    let ranked_lifecycle = Arc::new(std::sync::Mutex::new(
        RankedSessionLifecycle::awaiting_prepared_inputs(),
    ));
    let ranked_lifecycle_for_thread = Arc::clone(&ranked_lifecycle);
    let (ranked_setup_tx, mut ranked_setup_rx) = unbounded_channel();
    let session_metadata_for_thread = Arc::clone(&session_metadata);
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
                    session_metadata_for_thread,
                    ranked_lifecycle_for_thread,
                    &mut ranked_setup_rx,
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
        session_metadata,
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
pub(super) struct ClientSession {
    // Held so the QUIC connection stays open for the streams' lifetime.
    pub(super) _conn: Connection,
    pub(super) send: SendStream,
    pub(super) recv: RecvStream,
    pub(super) protocol: crate::multiplayer::client_protocol::ClientHandshake,
}

#[derive(Debug)]
pub(super) enum HandshakePrelude {
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
pub(super) enum InitialHandshake {
    Welcomed { seat: PlayerId, mission_seed: u64 },
    ContentOffered { full_mod_sha256: [u8; 32] },
}

/// One round of (connect → open stream → Hello → Welcome).  Used both
/// for the initial handshake and for the auto-retry path after
/// disconnects.
pub(super) async fn handshake_async(
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
        crate::multiplayer::client_protocol::ClientHandshake::new(server_addr.id.to_string(), None);
    let action = protocol.receive(message)?;
    let session = ClientSession {
        _conn: conn,
        send,
        recv,
        protocol,
    };
    match action {
        crate::multiplayer::client_protocol::HandshakeAction::Welcome(welcome) => {
            Ok(HandshakePrelude::Welcome { session, welcome })
        }
        crate::multiplayer::client_protocol::HandshakeAction::PrepareContent(offer) => {
            Ok(HandshakePrelude::Content { session, offer })
        }
    }
}

pub(super) async fn handshake_or_cancel(
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

pub(super) async fn read_welcome(
    mut session: ClientSession,
) -> Result<(ClientSession, WelcomeData), String> {
    session.protocol.content_ready()?;
    let message = read_frame_bounded_with_timeout(
        &mut session.recv,
        InboundFramePolicy::ServerToClient,
        HANDSHAKE_FRAME_TIMEOUT,
        "post-content Welcome",
    )
    .await?;
    match session.protocol.receive(message)? {
        crate::multiplayer::client_protocol::HandshakeAction::Welcome(welcome) => {
            Ok((session, welcome))
        }
        crate::multiplayer::client_protocol::HandshakeAction::PrepareContent(_) => {
            unreachable!("post-content phase only accepts Welcome")
        }
    }
}

/// Complete first-use admission under game/menu control. The transport
/// validates every wire invariant and does not send `ContentReady` itself;
/// that acknowledgement must come from the consumer after durable staging,
/// full-package hash validation, and deterministic mount preparation.
pub(super) enum ContentAdmissionCompletion {
    Join(ClientSession, WelcomeData),
    Prepared,
}

pub(super) async fn complete_content_admission(
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
        crate::multiplayer::client_gameplay::deliver(
            incoming_tx,
            NetEvent::ContentChunk {
                full_mod_sha256,
                offset,
                total_bytes,
                bytes,
            },
        )?;
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
            Ok(ContentAdmissionCompletion::Join(session, welcome))
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
            Ok(ContentAdmissionCompletion::Prepared)
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
            Err(format!(
                "downloaded host content failed local admission: {reason}"
            ))
        }
        other => Err(format!(
            "expected local ContentReady/ContentPrepared/ContentReject, got {other:?}"
        )),
    }
}

/// A reconnect may bypass byte transfer only for the identical offer that
/// this same live client session already admitted and mounted. Any content
/// change or content/no-content downgrade is a hard reconnect failure.
pub(super) async fn resolve_reconnect_prelude(
    prelude: HandshakePrelude,
    admitted: Option<&robin_engine::multiplayer::DistributedModOffer>,
) -> Result<(ClientSession, WelcomeData), String> {
    let offered = match &prelude {
        HandshakePrelude::Welcome { .. } => None,
        HandshakePrelude::Content { offer, .. } => Some(offer),
    };
    crate::multiplayer::client_protocol::validate_reconnect_content(offered, admitted)?;
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

/// Drive one connection until it ends, then auto-reconnect with
/// exponential backoff.  Returns when the game loop drops the
/// outgoing queue (`host.net` dropped) or shutdown is requested.
pub(super) async fn run_client_io_async(
    transport_key: SecretKey,
    durable_ranked_key: Option<SecretKey>,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    session_metadata: Arc<Mutex<Option<ClientSessionMetadata>>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_setup_rx: &mut UnboundedReceiver<
        Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    >,
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
        session_metadata,
        ranked_lifecycle,
        ranked_setup_rx,
        content_offer_shared,
        initial_handshake_tx,
        cancellation,
    )
    .await;

    endpoint.close().await;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_client_io_inner(
    endpoint: &Endpoint,
    durable_ranked_key: Option<SecretKey>,
    server_addr: EndpointAddr,
    nickname: String,
    incoming_tx: Sender<NetEvent>,
    outgoing_async_rx: &mut UnboundedReceiver<NetOutbound>,
    session_metadata: Arc<Mutex<Option<ClientSessionMetadata>>>,
    ranked_lifecycle: SharedRankedSessionLifecycle,
    ranked_setup_rx: &mut UnboundedReceiver<
        Option<crate::leaderboard_ranked_session::OfficialRankedSessionSetupV1>,
    >,
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
            if crate::multiplayer::client_gameplay::deliver(
                &incoming_tx,
                NetEvent::ContentOffer(offer.clone()),
            )
            .is_err()
            {
                return;
            }
            if initial_handshake_tx
                .send(Ok(InitialHandshake::ContentOffered {
                    full_mod_sha256: offer.full_mod_sha256,
                }))
                .is_err()
            {
                return;
            }
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
    let admitted_session =
        match ClientSessionMetadata::from_welcome(&welcome, admitted_offer.clone()) {
            Ok(session) => session,
            Err(error) => {
                let _ = initial_handshake_tx.send(Err(error.clone()));
                let _ = incoming_tx.send(NetEvent::Fatal(error));
                return;
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

    *session_metadata.lock() = Some(admitted_session);
    *content_offer_shared.lock() = admitted_offer.clone();
    if admitted_offer.is_none() {
        if initial_handshake_tx
            .send(Ok(InitialHandshake::Welcomed {
                seat: your_seat,
                mission_seed,
            }))
            .is_err()
        {
            return;
        }
    }
    if crate::multiplayer::client_gameplay::deliver_lifecycle(
        &incoming_tx,
        [
            NetEvent::AssignedLocalSeat(your_seat),
            NetEvent::MissionConfig {
                mission_id: mission_id.clone(),
                rng_seed: mission_seed,
                sim_config,
                speech_timing_locale: speech_timing_locale.clone(),
            },
        ],
    )
    .is_err()
    {
        return;
    }
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
                if crate::multiplayer::client_gameplay::deliver_lifecycle(
                    &incoming_tx,
                    [
                        NetEvent::Note(format!("disconnected: {reason}; reconnecting...")),
                        NetEvent::Disconnected,
                    ],
                )
                .is_err()
                {
                    return;
                }
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
                    // Reconnect validation proved the published identity is unchanged.
                    if crate::multiplayer::client_gameplay::deliver_lifecycle(
                        &incoming_tx,
                        [
                            NetEvent::Reconnected,
                            NetEvent::AssignedLocalSeat(new_seat),
                            NetEvent::MissionConfig {
                                mission_id: new_mission_id,
                                rng_seed: new_seed,
                                sim_config: new_config,
                                speech_timing_locale: new_speech_timing_locale,
                            },
                        ],
                    )
                    .is_err()
                    {
                        return;
                    }
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
pub(super) enum SessionEnd {
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
pub(super) struct ClientRankedTransportContext {
    pub(super) local_seat: PlayerId,
    pub(super) local_transport_endpoint: [u8; 32],
    pub(super) authenticated_host_endpoint: [u8; 32],
    pub(super) durable_ranked_key: Option<SecretKey>,
    pub(super) lifecycle: SharedRankedSessionLifecycle,
    pub(super) join_state: SharedClientRankedJoinState,
    pub(super) setup_state: Arc<AtomicU8>,
    pub(super) response_tx: UnboundedSender<RankedJoinResponse>,
}

/// Throw away commands queued for a transport session whose prediction
/// future has been abandoned. Replaying them after the next handshake would
/// apply pre-disconnect input on top of the authoritative replacement
/// snapshot.
pub(super) fn discard_session_outbound(outgoing_rx: &mut UnboundedReceiver<NetOutbound>) -> usize {
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
pub(super) async fn run_session_async(
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

pub(super) async fn wait_for_cancel(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
}

/// Sleep for a reconnect backoff, returning early when shutdown begins.
pub(super) async fn sleep_or_cancel(duration: Duration, cancellation: &AtomicBool) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = wait_for_cancel(cancellation) => true,
    }
}

pub(super) fn downgrade_client_ranked(
    context: &ClientRankedTransportContext,
    reason: RankedBrowseOnlyReason,
    detail: impl Into<String>,
) {
    let detail = detail.into();
    ranked_lifecycle_lock(&context.lifecycle).downgrade(detail.clone());
    tracing::warn!(?reason, %detail, "client ranked admission downgraded; gameplay remains available");
}

pub(super) fn handle_client_ranked_setup(
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
                crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
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
        .and_then(|bytes| {
            crate::multiplayer::RankedSessionConfigDocument::new(bytes).map_err(str::to_string)
        });
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
                    crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
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

pub(super) fn queue_client_ranked_response(
    context: &ClientRankedTransportContext,
    response: RankedJoinResponse,
) -> Result<(), String> {
    context.join_state.authorize_response(&response)?;
    context
        .response_tx
        .send(response)
        .map_err(|_| "ranked response writer queue is closed".to_string())
}

pub(super) fn respond_to_ranked_challenge(
    context: &ClientRankedTransportContext,
    challenge: RankedJoinChallenge,
) -> Result<(), String> {
    let Some(durable_key) = context.durable_ranked_key.as_ref() else {
        return queue_client_ranked_response(
            context,
            RankedJoinResponse::Unavailable(
                crate::multiplayer::RankedJoinUnavailableReason::DurableIdentityUnavailable,
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

pub(super) fn handle_delivered_ranked_challenge(
    context: &ClientRankedTransportContext,
    challenge: RankedJoinChallenge,
) {
    if let Err(error) = respond_to_ranked_challenge(context, challenge) {
        let unavailable = RankedJoinResponse::Unavailable(
            crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
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

pub(super) fn accept_client_ranked_join(
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
    let participant_claims = crate::multiplayer::decode_ranked_participant_roster(
        &accepted.participant_roster,
        &genesis,
    )?;
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

pub(super) fn handle_client_wire_msg(
    incoming_tx: &Sender<NetEvent>,
    leaderboard_cosign_state: &SharedClientLeaderboardCoSignState,
    ranked_context: Option<&ClientRankedTransportContext>,
    msg: NetMsg,
) -> Result<(), String> {
    let Some(msg) = crate::multiplayer::client_gameplay::forward(msg, incoming_tx)? else {
        return Ok(());
    };
    match msg {
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
                    if let Err(error) = queue_client_ranked_response(
                        context,
                        RankedJoinResponse::Unavailable(
                            crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
                        ),
                    ) {
                        // Native permits browse-only play even if no ranked
                        // challenge was issued, so an unavailable response may
                        // not be authorized. The local downgrade below remains
                        // required; it cannot be lost along with this response.
                        tracing::warn!(%error, "could not notify host of premature ranked BeginSim");
                    }
                    context
                        .join_state
                        .mark_browse_only(RankedBrowseOnlyReason::RankedProtocolViolation)?;
                    downgrade_client_ranked(
                        context,
                        RankedBrowseOnlyReason::RankedProtocolViolation,
                        "host released simulation before ranked admission or browse-only resolution",
                    );
                    crate::multiplayer::client_gameplay::deliver(
                        incoming_tx,
                        NetEvent::RankedBrowseOnly {
                            reason: RankedBrowseOnlyReason::RankedProtocolViolation,
                        },
                    )?;
                }
            }
            crate::multiplayer::client_gameplay::deliver(
                incoming_tx,
                NetEvent::BeginSim {
                    frame,
                    start_epoch_ms,
                },
            )?;
        }
        NetMsg::ModalProposal { .. } => {
            return Err("server sent a client-only modal proposal".to_string());
        }
        NetMsg::ReconnectRequired { reason } => {
            return Err(format!("host requires a full-snapshot reconnect: {reason}"));
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
                            crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionUnavailable,
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
                            crate::multiplayer::RankedJoinUnavailableReason::LocalRankedSessionMismatch,
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
                    crate::multiplayer::decode_ranked_participant_roster(&document, &genesis)?;
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

pub(super) async fn send_client_outgoing(
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
pub(super) fn client_gameplay_wire_msg(outgoing: NetOutbound) -> Result<NetMsg, String> {
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
